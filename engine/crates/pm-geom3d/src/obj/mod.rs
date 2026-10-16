//! Wavefront OBJ (sections 13 and 25).
//!
//! - [`parse`] is the kernel's `parseObj` (`renderer/src/kernel/obj.ts`):
//!   legacy `mesh` layers keep their OBJ assets and draw this geometry,
//!   centered and normalized to a unit bounding sphere.
//! - [`prepare_import`] (feature `import`) is `prepareModelImport`
//!   (`core/scene3d/import-bundle.ts`): an OBJ chosen with its MTL and
//!   texture files becomes one GLB, as three.js r180's `OBJLoader`,
//!   `MTLLoader`, and `GLTFExporter` make it, with Phong materials turned
//!   standard and bump maps turned into normal maps.

use crate::gltf::{Error, Result};
use crate::js;

#[cfg(feature = "import")]
mod loader;
#[cfg(feature = "import")]
mod pack;

#[cfg(feature = "import")]
pub use pack::{ImportFile, Prepared, prepare_import};

/// `ObjMesh`.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjMesh {
    /// Centered and normalized to a unit bounding sphere, three per vertex.
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub texcoords: Vec<f32>,
    pub vertex_count: usize,
    pub triangle_count: usize,
    pub source_vertex_count: usize,
    pub bounds: ObjBounds,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObjBounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub center: [f64; 3],
    pub radius: f64,
}

pub const MAX_SOURCE_VERTICES: usize = 1_000_000;
pub const MAX_TRIANGLES: usize = 500_000;

fn err<T>(m: impl Into<String>) -> Result<T> {
    Err(Error(m.into()))
}

fn finite(value: &str, line: usize) -> Result<f64> {
    let n = js::number(value);
    if n.is_finite() {
        Ok(n)
    } else {
        err(format!("OBJ line {line}: expected a finite number"))
    }
}

fn resolve_index(raw: Option<&str>, length: usize, line: usize, label: &str) -> Result<Option<usize>> {
    let Some(raw) = raw.filter(|r| !r.is_empty()) else {
        return Ok(None);
    };
    let value = js::number(raw);
    if !value.is_finite() || value.fract() != 0.0 || value == 0.0 {
        return err(format!("OBJ line {line}: invalid {label} index"));
    }
    let index = if value > 0.0 {
        value - 1.0
    } else {
        length as f64 + value
    };
    if index < 0.0 || index >= length as f64 {
        return err(format!("OBJ line {line}: {label} index is out of range"));
    }
    Ok(Some(index as usize))
}

type V3 = [f64; 3];

fn face_normal(a: V3, b: V3, c: V3) -> V3 {
    let (abx, aby, abz) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
    let (acx, acy, acz) = (c[0] - a[0], c[1] - a[1], c[2] - a[2]);
    let x = aby * acz - abz * acy;
    let y = abz * acx - abx * acz;
    let z = abx * acy - aby * acx;
    let length = js::hypot(&[x, y, z]);
    if length > 1e-12 {
        [x / length, y / length, z / length]
    } else {
        [0.0, 1.0, 0.0]
    }
}

#[derive(Clone, Copy)]
struct Corner {
    v: usize,
    vt: Option<usize>,
    vn: Option<usize>,
}

/// `parseObj`: v, vt, vn, and polygonal faces (fan-triangulated, negative
/// indices resolved); everything else is ignored.
pub fn parse(source: &str) -> Result<ObjMesh> {
    if js::trim(source).is_empty() {
        return err("OBJ file is empty");
    }
    let mut vertices: Vec<V3> = vec![];
    let mut normals: Vec<V3> = vec![];
    let mut texcoords: Vec<[f64; 2]> = vec![];
    let mut triangles: Vec<[Corner; 3]> = vec![];
    let text = source.replace("\r\n", "\n").replace('\r', "\n");
    for (index, raw) in text.split('\n').enumerate() {
        let n = index + 1;
        let line = js::trim(raw.split('#').next().unwrap_or(""));
        if line.is_empty() {
            continue;
        }
        let fields = js::split_ws(line);
        let (kind, fields) = (fields[0], &fields[1..]);
        match kind {
            "v" => {
                if fields.len() < 3 {
                    return err(format!("OBJ line {n}: vertex needs x, y, and z"));
                }
                vertices.push([finite(fields[0], n)?, finite(fields[1], n)?, finite(fields[2], n)?]);
                if vertices.len() > MAX_SOURCE_VERTICES {
                    return err("OBJ has too many vertices");
                }
            }
            "vn" => {
                if fields.len() < 3 {
                    return err(format!("OBJ line {n}: normal needs x, y, and z"));
                }
                let raw = [finite(fields[0], n)?, finite(fields[1], n)?, finite(fields[2], n)?];
                let length = js::hypot(&raw);
                normals.push(if length > 1e-12 {
                    [raw[0] / length, raw[1] / length, raw[2] / length]
                } else {
                    [0.0, 1.0, 0.0]
                });
            }
            "vt" => {
                if fields.len() < 2 {
                    return err(format!("OBJ line {n}: texture coordinate needs u and v"));
                }
                texcoords.push([finite(fields[0], n)?, finite(fields[1], n)?]);
            }
            "f" => {
                if fields.len() < 3 {
                    return err(format!("OBJ line {n}: face needs at least three vertices"));
                }
                let mut face = Vec::with_capacity(fields.len());
                for field in fields {
                    let mut parts = field.split('/');
                    let (v, vt, vn) = (parts.next(), parts.next(), parts.next());
                    let Some(v) = resolve_index(v, vertices.len(), n, "vertex")? else {
                        // parseObj asserts this index is present.
                        return err(format!("OBJ line {n}: invalid vertex index"));
                    };
                    face.push(Corner {
                        v,
                        vt: resolve_index(vt, texcoords.len(), n, "texture coordinate")?,
                        vn: resolve_index(vn, normals.len(), n, "normal")?,
                    });
                }
                for corner in 1..face.len() - 1 {
                    triangles.push([face[0], face[corner], face[corner + 1]]);
                    if triangles.len() > MAX_TRIANGLES {
                        return err("OBJ has too many triangles");
                    }
                }
            }
            _ => {}
        }
    }
    if vertices.is_empty() {
        return err("OBJ does not contain any vertices");
    }
    if triangles.is_empty() {
        return err("OBJ does not contain any faces");
    }
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for v in &vertices {
        for c in 0..3 {
            min[c] = js::min(min[c], v[c]);
            max[c] = js::max(max[c], v[c]);
        }
    }
    let center: V3 = std::array::from_fn(|c| (min[c] + max[c]) / 2.0);
    let mut radius = 0.0f64;
    for v in &vertices {
        radius = js::max(radius, js::hypot(&[v[0] - center[0], v[1] - center[1], v[2] - center[2]]));
    }
    if radius.is_nan() || radius <= 1e-12 {
        return err("OBJ geometry has no measurable size");
    }
    let mut positions = Vec::with_capacity(triangles.len() * 9);
    let mut out_normals = Vec::with_capacity(triangles.len() * 9);
    let mut out_texcoords = Vec::with_capacity(triangles.len() * 6);
    for t in &triangles {
        let fallback = face_normal(vertices[t[0].v], vertices[t[1].v], vertices[t[2].v]);
        for r in t {
            let v = vertices[r.v];
            for c in 0..3 {
                positions.push(((v[c] - center[c]) / radius) as f32);
            }
            let normal = r.vn.map_or(fallback, |i| normals[i]);
            out_normals.extend(normal.map(|x| x as f32));
            let uv = r.vt.map_or([0.0; 2], |i| texcoords[i]);
            out_texcoords.extend(uv.map(|x| x as f32));
        }
    }
    Ok(ObjMesh {
        positions,
        normals: out_normals,
        texcoords: out_texcoords,
        vertex_count: triangles.len() * 3,
        triangle_count: triangles.len(),
        source_vertex_count: vertices.len(),
        bounds: ObjBounds {
            min,
            max,
            center,
            radius,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_faces_like_parse_obj() {
        let m = parse("v 0 0 0\nv 2 0 0\nv 2 2 0\nv 0 2 0\nvt 0 0\nvn 0 0 5\nf 1/1/1 2 3 -1 # quad\n").unwrap();
        assert_eq!((m.vertex_count, m.triangle_count, m.source_vertex_count), (6, 2, 4));
        assert_eq!(m.bounds.center, [1.0, 1.0, 0.0]);
        assert_eq!(m.bounds.radius, 2f64.sqrt());
        assert_eq!(&m.normals[..3], &[0.0, 0.0, 1.0]);
        // A corner without vn takes the face normal.
        assert_eq!(&m.normals[3..6], &[0.0, 0.0, 1.0]);
        assert_eq!(&m.texcoords[..4], &[0.0, 0.0, 0.0, 0.0]);
        assert_eq!(parse("  \n").unwrap_err().0, "OBJ file is empty");
        assert_eq!(parse("v 0 0\n").unwrap_err().0, "OBJ line 1: vertex needs x, y, and z");
        assert_eq!(parse("v 0 0 x\n").unwrap_err().0, "OBJ line 1: expected a finite number");
        assert_eq!(parse("v 0 0 0\nf 1 2 3\n").unwrap_err().0, "OBJ line 2: vertex index is out of range");
        assert_eq!(parse("v 0 0 0\nf 1 1.5 1\n").unwrap_err().0, "OBJ line 2: invalid vertex index");
        assert_eq!(parse("v 0 0 0\n").unwrap_err().0, "OBJ does not contain any faces");
        assert_eq!(parse("v 0 0 0\nf 1 1 1\n").unwrap_err().0, "OBJ geometry has no measurable size");
    }
}
