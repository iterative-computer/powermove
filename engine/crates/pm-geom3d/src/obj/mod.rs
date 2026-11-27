//! Wavefront OBJ meshes as the kernel parses them for legacy mesh layers
//! (`kernel/obj.ts` `parseObj`): `v`, `vt`, `vn`, and polygonal `f` with
//! negative indices, faces triangulated as fans, the result centered and
//! scaled to a unit bounding sphere, flattened to one vertex per triangle
//! corner. Numbers follow JavaScript (`Number(string)`, `Math.hypot` as V8
//! computes it), so the f32 buffers equal the kernel's.
//!
//! With the `import` feature, [`prepare_import`] is `prepareModelImport`
//! (`core/scene3d/import-bundle.ts`): an OBJ chosen with its MTL and texture
//! files becomes one GLB, as three.js r180's `OBJLoader`, `MTLLoader`, and
//! `GLTFExporter` make it, with Phong materials turned standard and bump
//! maps turned into normal maps. It validates the geometry with
//! [`parse_obj`] first, as `prepareModelImport` calls `parseObj`.

#[cfg(feature = "import")]
mod loader;
#[cfg(feature = "import")]
mod pack;

#[cfg(feature = "import")]
pub use pack::{ImportFile, Prepared, prepare_import};

/// A parsed OBJ mesh (`ObjMesh`).
#[derive(Debug, Clone, PartialEq)]
pub struct ObjMesh {
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub texcoords: Vec<f32>,
    pub vertex_count: usize,
    pub triangle_count: usize,
    pub source_vertex_count: usize,
    pub center: [f64; 3],
    pub radius: f64,
}

const MAX_SOURCE_VERTICES: usize = 1_000_000;
const MAX_TRIANGLES: usize = 500_000;

/// JavaScript `WhiteSpace` and `LineTerminator`.
fn js_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{FEFF}'
}

fn decimal_literal(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let digits = |i: &mut usize| {
        let start = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i - start
    };
    let int = digits(&mut i);
    let mut frac = 0;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        frac = digits(&mut i);
    }
    if int + frac == 0 {
        return false;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        if digits(&mut i) == 0 {
            return false;
        }
    }
    i == b.len()
}

/// `Number(string)` (StringToNumber).
pub fn js_number(s: &str) -> f64 {
    let t = s.trim_matches(js_space);
    if t.is_empty() {
        return 0.0;
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    let radix = |p: &str, r: u32| -> f64 {
        if p.is_empty() {
            return f64::NAN;
        }
        let mut v = 0f64;
        for c in p.chars() {
            match c.to_digit(r) {
                Some(d) => v = v * f64::from(r) + f64::from(d),
                None => return f64::NAN,
            }
        }
        v
    };
    if t.len() > 2 && t.as_bytes()[0] == b'0' {
        match t.as_bytes()[1] {
            b'x' | b'X' => return radix(&t[2..], 16),
            b'o' | b'O' => return radix(&t[2..], 8),
            b'b' | b'B' => return radix(&t[2..], 2),
            _ => {}
        }
    }
    if decimal_literal(t) {
        t.parse::<f64>().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

/// `Math.hypot(...values)` as V8 computes it (scaled Kahan summation).
pub fn js_hypot(values: &[f64]) -> f64 {
    let mut max = 0f64;
    let mut nan = false;
    for v in values {
        let a = v.abs();
        if a.is_nan() {
            nan = true;
        } else if a > max {
            max = a;
        }
    }
    if max == f64::INFINITY {
        return f64::INFINITY;
    }
    if nan {
        return f64::NAN;
    }
    if max == 0.0 {
        return 0.0;
    }
    let mut sum = 0f64;
    let mut compensation = 0f64;
    for v in values {
        let n = v.abs() / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    sum.sqrt() * max
}

fn finite(value: &str, line: usize) -> Result<f64, String> {
    let n = js_number(value);
    if n.is_finite() {
        Ok(n)
    } else {
        Err(format!("OBJ line {line}: expected a finite number"))
    }
}

fn resolve_index(
    raw: Option<&str>,
    length: usize,
    line: usize,
    label: &str,
) -> Result<Option<usize>, String> {
    let Some(raw) = raw.filter(|r| !r.is_empty()) else {
        return Ok(None);
    };
    let value = js_number(raw);
    if !(value.is_finite() && value.trunc() == value) || value == 0.0 {
        return Err(format!("OBJ line {line}: invalid {label} index"));
    }
    let index = if value > 0.0 {
        value - 1.0
    } else {
        length as f64 + value
    };
    if index < 0.0 || index >= length as f64 {
        return Err(format!("OBJ line {line}: {label} index is out of range"));
    }
    Ok(Some(index as usize))
}

fn face_normal(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 3] {
    let (abx, aby, abz) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
    let (acx, acy, acz) = (c[0] - a[0], c[1] - a[1], c[2] - a[2]);
    let x = aby * acz - abz * acy;
    let y = abz * acx - abx * acz;
    let z = abx * acy - aby * acx;
    let length = js_hypot(&[x, y, z]);
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

/// `parseObj(source)`.
pub fn parse_obj(source: &str) -> Result<ObjMesh, String> {
    if source.trim_matches(js_space).is_empty() {
        return Err("OBJ file is empty".into());
    }
    let mut vertices: Vec<[f64; 3]> = Vec::new();
    let mut normals: Vec<[f64; 3]> = Vec::new();
    let mut texcoords: Vec<[f64; 2]> = Vec::new();
    let mut triangles: Vec<[Corner; 3]> = Vec::new();
    let text = source.replace("\r\n", "\n").replace('\r', "\n");
    for (index, raw) in text.split('\n').enumerate() {
        let line_number = index + 1;
        let line = raw.split('#').next().unwrap_or("").trim_matches(js_space);
        if line.is_empty() {
            continue;
        }
        let mut fields: Vec<&str> = line.split(js_space).filter(|f| !f.is_empty()).collect();
        let kind = fields.remove(0);
        match kind {
            "v" => {
                if fields.len() < 3 {
                    return Err(format!("OBJ line {line_number}: vertex needs x, y, and z"));
                }
                vertices.push([
                    finite(fields[0], line_number)?,
                    finite(fields[1], line_number)?,
                    finite(fields[2], line_number)?,
                ]);
                if vertices.len() > MAX_SOURCE_VERTICES {
                    return Err("OBJ has too many vertices".into());
                }
            }
            "vn" => {
                if fields.len() < 3 {
                    return Err(format!("OBJ line {line_number}: normal needs x, y, and z"));
                }
                let r = [
                    finite(fields[0], line_number)?,
                    finite(fields[1], line_number)?,
                    finite(fields[2], line_number)?,
                ];
                let length = js_hypot(&r);
                normals.push(if length > 1e-12 {
                    [r[0] / length, r[1] / length, r[2] / length]
                } else {
                    [0.0, 1.0, 0.0]
                });
            }
            "vt" => {
                if fields.len() < 2 {
                    return Err(format!(
                        "OBJ line {line_number}: texture coordinate needs u and v"
                    ));
                }
                texcoords.push([
                    finite(fields[0], line_number)?,
                    finite(fields[1], line_number)?,
                ]);
            }
            "f" => {
                if fields.len() < 3 {
                    return Err(format!(
                        "OBJ line {line_number}: face needs at least three vertices"
                    ));
                }
                let mut face = Vec::with_capacity(fields.len());
                for field in &fields {
                    let mut parts = field.split('/');
                    let (v, vt, vn) = (parts.next(), parts.next(), parts.next());
                    let v = resolve_index(v, vertices.len(), line_number, "vertex")?;
                    // `resolveIndex(...)!`: an empty vertex index is `null`
                    // and fails when used, as a TypeError would.
                    let Some(v) = v else {
                        return Err(format!("OBJ line {line_number}: invalid vertex index"));
                    };
                    face.push(Corner {
                        v,
                        vt: resolve_index(vt, texcoords.len(), line_number, "texture coordinate")?,
                        vn: resolve_index(vn, normals.len(), line_number, "normal")?,
                    });
                }
                for corner in 1..face.len() - 1 {
                    triangles.push([face[0], face[corner], face[corner + 1]]);
                    if triangles.len() > MAX_TRIANGLES {
                        return Err("OBJ has too many triangles".into());
                    }
                }
            }
            _ => {}
        }
    }
    if vertices.is_empty() {
        return Err("OBJ does not contain any vertices".into());
    }
    if triangles.is_empty() {
        return Err("OBJ does not contain any faces".into());
    }
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for v in &vertices {
        for k in 0..3 {
            min[k] = js_min(min[k], v[k]);
            max[k] = js_max(max[k], v[k]);
        }
    }
    let center = [
        (min[0] + max[0]) / 2.0,
        (min[1] + max[1]) / 2.0,
        (min[2] + max[2]) / 2.0,
    ];
    let mut radius = 0f64;
    for v in &vertices {
        radius = js_max(
            radius,
            js_hypot(&[v[0] - center[0], v[1] - center[1], v[2] - center[2]]),
        );
    }
    if radius.is_nan() || radius <= 1e-12 {
        return Err("OBJ geometry has no measurable size".into());
    }
    let mut positions = Vec::with_capacity(triangles.len() * 9);
    let mut out_normals = Vec::with_capacity(triangles.len() * 9);
    let mut out_uvs = Vec::with_capacity(triangles.len() * 6);
    for t in &triangles {
        let fallback = face_normal(vertices[t[0].v], vertices[t[1].v], vertices[t[2].v]);
        for r in t {
            let v = vertices[r.v];
            for k in 0..3 {
                positions.push(((v[k] - center[k]) / radius) as f32);
            }
            let n = r.vn.map_or(fallback, |i| normals[i]);
            out_normals.extend(n.iter().map(|x| *x as f32));
            let uv = r.vt.map_or([0.0, 0.0], |i| texcoords[i]);
            out_uvs.extend(uv.iter().map(|x| *x as f32));
        }
    }
    Ok(ObjMesh {
        positions,
        normals: out_normals,
        texcoords: out_uvs,
        vertex_count: triangles.len() * 3,
        triangle_count: triangles.len(),
        source_vertex_count: vertices.len(),
        center,
        radius,
    })
}

/// `Math.min(a, b)` (NaN wins).
fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

/// `Math.max(a, b)` (NaN wins).
fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_follow_javascript() {
        assert_eq!(js_number(" 1.5 "), 1.5);
        assert_eq!(js_number(".5"), 0.5);
        assert_eq!(js_number("5."), 5.0);
        assert_eq!(js_number("-2e3"), -2000.0);
        assert_eq!(js_number("0x10"), 16.0);
        assert!(js_number("inf").is_nan());
        assert!(js_number("1,5").is_nan());
        assert_eq!(js_number("-Infinity"), f64::NEG_INFINITY);
    }

    #[test]
    fn hypot_is_exact_on_pythagorean_triples() {
        assert_eq!(js_hypot(&[3.0, 4.0]), 5.0);
        assert_eq!(js_hypot(&[0.0, 0.0, 0.0]), 0.0);
    }

    #[test]
    fn quads_triangulate_as_fans_and_normalize() {
        let m = parse_obj("v 0 0 0\nv 2 0 0\nv 2 2 0\nv 0 2 0\nf 1 2 3 4\n").unwrap();
        assert_eq!(m.triangle_count, 2);
        assert_eq!(m.vertex_count, 6);
        assert_eq!(m.center, [1.0, 1.0, 0.0]);
        assert!((m.radius - 2f64.sqrt()).abs() < 1e-15);
        assert_eq!(&m.normals[..3], &[0.0, 0.0, 1.0]);
        let neg = parse_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf -3 -2 -1\n").unwrap();
        assert_eq!(neg.triangle_count, 1);
        assert!(parse_obj("v 0 0 0\nf 1 2 3\n").is_err());
        assert!(parse_obj("  ").is_err());
    }
}
