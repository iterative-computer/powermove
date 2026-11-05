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
                "obj" | "glb" | "gltf" | "ply" | "splat" | "step" | "stp" | "iges" | "igs" | "brep"
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
