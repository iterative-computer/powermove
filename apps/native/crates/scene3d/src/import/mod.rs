//! Bounded imports of explicitly supplied bytes. No URL or sibling-file reads.
pub mod archive;
pub mod cad;
mod obj;
pub mod splats;
use crate::{Error, Result, fail};
pub use obj::ImportFile;
use pm_geom3d::{gltf, model_import::glb::GlbWriter};
use serde_json::json;
pub const MAX_BYTES: usize = 512 * 1024 * 1024;
pub const MAX_TRIANGLES: usize = 10_000_000;
#[derive(Clone, Debug)]
pub struct Prepared {
    pub name: String,
    pub bytes: Vec<u8>,
    pub original: Vec<ImportFile>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Model {
    Gltf(Box<gltf::Model>),
    Obj(pm_geom3d::obj::ObjMesh),
    Splats(splats::Data),
}
fn ext(name: &str) -> String {
    name.rsplit('.').next().unwrap_or("").to_ascii_lowercase()
}
pub fn load(name: &str, bytes: &[u8]) -> Result<Model> {
    if bytes.len() > MAX_BYTES {
        return fail("Model exceeds 512 MB");
    }
    match ext(name).as_str() {
        "glb" | "gltf" => gltf::load_with(
            bytes,
            gltf::Limits {
                bytes: MAX_BYTES,
                accessor_count: (MAX_TRIANGLES * 3) as u64,
                vertices: (MAX_TRIANGLES * 3) as u64,
                nodes: 65536,
                triangles: MAX_TRIANGLES,
            },
        )
        .and_then(|m| {
            if let Some(g) = m
                .geometries
                .iter()
                .find(|g| g.undrawable.is_some() || !g.short_attributes.is_empty())
            {
                return Err(gltf::Error(format!(
                    "Malformed model geometry: {:?}; {:?}",
                    g.undrawable, g.short_attributes
                )));
            }
            Ok(Model::Gltf(Box::new(m)))
        })
        .map_err(|e| Error(e.to_string())),
        "obj" => {
            pm_geom3d::obj::parse_obj(std::str::from_utf8(bytes).map_err(|e| Error(e.to_string()))?)
                .map(Model::Obj)
                .map_err(Error)
        }
        "fbx" => load("converted.glb", &convert_fbx(bytes)?),
        "splat" => splats::parse_splat(bytes).map(Model::Splats),
        "ply" => splats::parse_ply(bytes).map(Model::Splats),
        "blend" => fail("Blender files are not supported"),
        _ => fail("Unsupported model format"),
    }
}
pub fn prepare(files: Vec<ImportFile>, cad: Option<&dyn cad::Tessellator>) -> Result<Prepared> {
    let total = files.iter().try_fold(0usize, |a, f| {
        a.checked_add(f.bytes.len())
            .ok_or_else(|| Error("Model size overflow".into()))
    })?;
    if files.is_empty() || files.len() > 128 || total > MAX_BYTES {
        return fail("Choose at most 128 model files, 512 MB in total");
    }
    let mut names = std::collections::HashSet::new();
    for f in &files {
        let leaf = f
            .name
            .replace('\\', "/")
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_lowercase();
        if !names.insert(leaf) {
            return fail("Duplicate selected file name");
        }
    }
    let models = files
        .iter()
        .enumerate()
        .filter(|(_, f)| {
            matches!(
                ext(&f.name).as_str(),
                "obj"
                    | "glb"
                    | "gltf"
                    | "fbx"
                    | "ply"
                    | "splat"
                    | "step"
                    | "stp"
                    | "iges"
                    | "igs"
                    | "brep"
            )
        })
        .collect::<Vec<_>>();
    if models.len() != 1 {
        return fail("Choose one model at a time with its material and texture files");
    }
    let (index, f) = models[0];
    let e = ext(&f.name);
    let (name, bytes) = if let Some(format) = cad::Format::from_extension(&e) {
        let converter =
            cad.ok_or_else(|| Error("Native OpenCascade CAD worker is not installed".into()))?;
        let bytes = converter.convert(format, &f.bytes, cad::Settings::default())?;
        load("converted.glb", &bytes)?;
        (replace_ext(&f.name, "glb"), bytes)
    } else if e == "fbx" {
        let converted = convert_fbx(&f.bytes)?;
        load("converted.glb", &converted)?;
        (replace_ext(&f.name, "glb"), converted)
    } else if e == "obj" && files.len() > 1 {
        match obj::prepare_import(&files).map_err(|e| Error(e.to_string()))? {
            obj::Prepared::Glb { name, bytes } => {
                load(&name, &bytes)?;
                (name, bytes)
            }
            obj::Prepared::Unchanged(_) => (f.name.clone(), f.bytes.clone()),
        }
    } else {
        load(&f.name, &f.bytes)?;
        (f.name.clone(), f.bytes.clone())
    };
    let _ = index;
    Ok(Prepared {
        name,
        bytes,
        original: files,
    })
}
/// The shared importer never opens sibling files. Match its Electron host's
/// stack allowance; source admission and rendering call this on workers.
fn convert_fbx(bytes: &[u8]) -> Result<Vec<u8>> {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("native-fbx-import".into())
            .stack_size(8 * 1024 * 1024)
            .spawn_scoped(scope, || pm_import_fbx::convert(bytes))
            .map_err(|e| Error(format!("Cannot start FBX import: {e}")))?
            .join()
            .map_err(|_| Error("FBX import worker stopped".into()))?
            .map(|imported| imported.glb)
            .map_err(Error)
    })
}
fn replace_ext(name: &str, e: &str) -> String {
    format!("{}.{e}", name.rsplit_once('.').map(|x| x.0).unwrap_or(name))
}
/// A neutral GLB writer for native CAD adapters and synthetic import tests. CAD
/// conversion itself uses XDE to retain part hierarchy and per-face materials.
pub fn triangle_glb(positions: &[[f32; 3]], indices: &[u32]) -> Vec<u8> {
    let mut w = GlbWriter::default();
    let p = w.floats(positions, true);
    let i = w.indices(indices);
    w.finish(json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":p},"indices":i}]}]}).as_object().unwrap().clone())
}

/// Flat quads facing +z, one primitive and named material each, in order
/// (material index = position): a minimal multi-part model.
pub fn quads_glb(parts: &[(&str, [[f32; 3]; 4])]) -> Vec<u8> {
    let mut w = GlbWriter::default();
    let quad = w.indices(&[0, 1, 2, 0, 2, 3]);
    let primitives: Vec<_> = parts
        .iter()
        .enumerate()
        .map(|(i, (_, corners))| {
            let p = w.floats(corners, true);
            json!({"attributes":{"POSITION":p},"indices":quad,"material":i})
        })
        .collect();
    let materials: Vec<_> = parts.iter().map(|(name, _)| json!({"name":name})).collect();
    w.finish(json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],"materials":materials,"meshes":[{"primitives":primitives}]}).as_object().unwrap().clone())
}
/// The material slots a model's parts can override, by their source index
/// (glTF material index; FBX converts to glTF): `(index, name)` in index
/// order, named "Material N" when the file leaves the name empty. OBJ parts
/// carry no source index, so OBJ has no slots.
pub fn material_slots(model: &Model) -> Vec<(usize, String)> {
    let Model::Gltf(m) = model else {
        return vec![];
    };
    let mut slots: std::collections::BTreeMap<usize, String> = Default::default();
    for (derived, source) in m.material_sources.iter().enumerate() {
        if let Some(index) = source {
            let name = m
                .materials
                .get(derived)
                .map(|mat| mat.name.trim().to_owned())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| format!("Material {}", index + 1));
            slots.entry(*index).or_insert(name);
        }
    }
    slots.into_iter().collect()
}
/// `material_slots` as stored on an imported asset (`native3dMaterials`).
pub fn material_slots_json(model: &Model) -> serde_json::Value {
    json!(
        material_slots(model)
            .into_iter()
            .map(|(index, name)| json!({"index": index, "name": name}))
            .collect::<Vec<_>>()
    )
}
/// Slots of an asset imported before `native3dMaterials` was recorded, read
/// from its file: a GLB's or glTF's JSON material names (cheap: no geometry),
/// else a full load (FBX).
pub fn material_slots_from_file(name: &str, bytes: &[u8]) -> Vec<(usize, String)> {
    let json_text: Option<&[u8]> = match ext(name).as_str() {
        "gltf" => Some(bytes),
        "glb" if bytes.len() >= 20 && &bytes[0..4] == b"glTF" => {
            let len = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
            (&bytes[16..20] == b"JSON" && bytes.len() >= 20 + len).then(|| &bytes[20..20 + len])
        }
        _ => None,
    };
    if let Some(text) = json_text {
        let Ok(doc) = serde_json::from_slice::<serde_json::Value>(text) else {
            return vec![];
        };
        return doc["materials"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(i, m)| {
                let name = m["name"].as_str().map(str::trim).filter(|n| !n.is_empty());
                (i, name.map_or_else(|| format!("Material {}", i + 1), str::to_owned))
            })
            .collect();
    }
    load(name, bytes).map(|m| material_slots(&m)).unwrap_or_default()
}

#[cfg(test)]
mod slot_tests {
    #[test]
    fn a_glb_lists_its_material_slots_by_source_index() {
        let json = br#"{"asset":{"version":"2.0"},"materials":[{"name":"Aluminium"},{"name":" "},{"name":"Glass"}]}"#;
        let mut padded = json.to_vec();
        while padded.len() % 4 != 0 {
            padded.push(b' ');
        }
        let mut glb = b"glTF".to_vec();
        glb.extend(2u32.to_le_bytes());
        glb.extend(((20 + padded.len()) as u32).to_le_bytes());
        glb.extend((padded.len() as u32).to_le_bytes());
        glb.extend(b"JSON");
        glb.extend(&padded);
        let slots = super::material_slots_from_file("Product.glb", &glb);
        assert_eq!(
            slots,
            vec![
                (0, "Aluminium".to_owned()),
                (1, "Material 2".to_owned()),
                (2, "Glass".to_owned())
            ]
        );
        assert!(super::material_slots_from_file("mesh.obj", b"v 0 0 0").is_empty());
    }
}
/// A GLB's or glTF's material slots with the values Properties shows before a
/// part is edited (sRGB hex colours; iridescence as film thickness in nm):
/// `[{index, name, values:{color, metallic, roughness, clearcoat,
/// transmission, ior, emissionColor, emissionIntensity, iridescence}}]`.
pub fn material_slot_values_from_file(name: &str, bytes: &[u8]) -> Vec<serde_json::Value> {
    let doc: Option<serde_json::Value> = match ext(name).as_str() {
        "gltf" => serde_json::from_slice(bytes).ok(),
        "glb" if bytes.len() >= 20 && &bytes[0..4] == b"glTF" => {
            let len = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
            (&bytes[16..20] == b"JSON" && bytes.len() >= 20 + len)
                .then(|| serde_json::from_slice(&bytes[20..20 + len]).ok())
                .flatten()
        }
        _ => None,
    };
    let Some(doc) = doc else {
        return material_slots_from_file(name, bytes)
            .into_iter()
            .map(|(index, name)| json!({"index": index, "name": name, "values": {}}))
            .collect();
    };
    let hex = |c: &serde_json::Value| {
        let channel = |i: usize| {
            let l = c[i].as_f64().unwrap_or(1.).clamp(0., 1.);
            let s = if l <= 0.0031308 { 12.92 * l } else { 1.055 * l.powf(1. / 2.4) - 0.055 };
            (s * 255.).round().clamp(0., 255.) as u8
        };
        format!("#{:02X}{:02X}{:02X}", channel(0), channel(1), channel(2))
    };
    doc["materials"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(i, m)| {
            let pbr = &m["pbrMetallicRoughness"];
            let x = &m["extensions"];
            let f = |v: &serde_json::Value, d: f64| v.as_f64().unwrap_or(d);
            let emissive = &m["emissiveFactor"];
            let emissive_on = emissive
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v.as_f64().unwrap_or(0.) > 0.));
            let film = if f(&x["KHR_materials_iridescence"]["iridescenceFactor"], 0.) > 0. {
                f(&x["KHR_materials_iridescence"]["iridescenceThicknessMaximum"], 400.)
            } else {
                0.
            };
            let name = m["name"].as_str().map(str::trim).filter(|n| !n.is_empty());
            json!({
                "index": i,
                "name": name.map_or_else(|| format!("Material {}", i + 1), str::to_owned),
                "values": {
                    "color": if pbr["baseColorFactor"].is_array() { hex(&pbr["baseColorFactor"]) } else { "#FFFFFF".into() },
                    "metallic": f(&pbr["metallicFactor"], 1.),
                    "roughness": f(&pbr["roughnessFactor"], 1.),
                    "clearcoat": f(&x["KHR_materials_clearcoat"]["clearcoatFactor"], 0.),
                    "transmission": f(&x["KHR_materials_transmission"]["transmissionFactor"], 0.),
                    "ior": f(&x["KHR_materials_ior"]["ior"], 1.5),
                    "emissionColor": if emissive_on { hex(emissive) } else { "#000000".into() },
                    "emissionIntensity": f(&x["KHR_materials_emissive_strength"]["emissiveStrength"], 1.),
                    "iridescence": film,
                }
            })
        })
        .collect()
}
