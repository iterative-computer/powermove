//! 3DGS binary PLY and the 32-byte .splat layout. Original bytes remain owned
//! by import admission. Normalization is a transform, never destructive edits.
use crate::{Error, Result, fail, navigation::*};
use std::collections::HashMap;
pub const MAX_SPLATS: usize = 16_000_000;
#[derive(Clone, Debug, PartialEq)]
pub struct Splat {
    pub center: [f32; 3],
    pub covariance: [f32; 6],
    pub rgba: [u8; 4],
}
#[derive(Clone, Debug, PartialEq)]
pub struct Data {
    pub splats: Vec<Splat>,
}
fn count(n: usize) -> Result<()> {
    if n == 0 || n > MAX_SPLATS {
        return fail("Splat count must be 1..=16000000");
    }
    Ok(())
}
fn finite(x: f64) -> Result<f64> {
    if !x.is_finite() || !(x as f32).is_finite() {
        return fail("Non-finite or overflowing splat data");
    }
    Ok(x)
}
fn covariance(scale: Vec3, q: [f64; 4]) -> Result<[f32; 6]> {
    for v in scale.into_iter().chain(q) {
        finite(v)?;
    }
    if scale.iter().any(|s| *s < 0.) {
        return fail("Negative splat scale");
    }
    let l = q.iter().map(|x| x * x).sum::<f64>().sqrt();
    let [w, x, y, z] = if l == 0. {
        [1., 0., 0., 0.]
    } else {
        q.map(|x| x / l)
    };
    let r = [
        [
            1. - 2. * (y * y + z * z),
            2. * (x * y - w * z),
            2. * (x * z + w * y),
        ],
        [
            2. * (x * y + w * z),
            1. - 2. * (x * x + z * z),
            2. * (y * z - w * x),
        ],
        [
            2. * (x * z - w * y),
            2. * (y * z + w * x),
            1. - 2. * (x * x + y * y),
        ],
    ];
    let m = r.map(|row| std::array::from_fn::<_, 3, _>(|i| row[i] * scale[i]));
    let d = |a: usize, b: usize| dot(m[a], m[b]);
    let c = [d(0, 0), d(0, 1), d(0, 2), d(1, 1), d(1, 2), d(2, 2)];
    for v in c {
        finite(v)?;
    }
    Ok(c.map(|x| x as f32))
}
fn f32_at(b: &[u8], o: usize) -> f64 {
    f32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as f64
}
pub fn parse_splat(bytes: &[u8]) -> Result<Data> {
    if !bytes.len().is_multiple_of(32) {
        return fail("Not a .splat file (32 bytes per splat)");
    }
    count(bytes.len() / 32)?;
    let mut splats = Vec::with_capacity(bytes.len() / 32);
    for b in bytes.chunks_exact(32) {
        let p = [f32_at(b, 0), f32_at(b, 4), f32_at(b, 8)];
        for v in p {
            finite(v)?;
        }
        let s = [f32_at(b, 12), f32_at(b, 16), f32_at(b, 20)];
        let q = std::array::from_fn(|i| (b[28 + i] as f64 - 128.) / 128.);
        splats.push(Splat {
            center: p.map(|x| x as f32),
            covariance: covariance(s, q)?,
            rgba: b[24..28].try_into().unwrap(),
        });
    }
    Ok(Data { splats })
}
#[derive(Clone, Copy)]
enum Ty {
    F32,
    F64,
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
}
impl Ty {
    fn parse(s: &str) -> Result<Self> {
        match s {
            "float" | "float32" => Ok(Self::F32),
            "double" | "float64" => Ok(Self::F64),
            "uchar" | "uint8" => Ok(Self::U8),
            "char" | "int8" => Ok(Self::I8),
            "ushort" | "uint16" => Ok(Self::U16),
            "short" | "int16" => Ok(Self::I16),
            "uint" | "uint32" => Ok(Self::U32),
            "int" | "int32" => Ok(Self::I32),
            _ => fail("Unsupported PLY scalar type"),
        }
    }
    fn size(self) -> usize {
        match self {
            Self::F64 => 8,
            Self::U8 | Self::I8 => 1,
            Self::U16 | Self::I16 => 2,
            _ => 4,
        }
    }
    fn read(self, b: &[u8], o: usize) -> f64 {
        match self {
            Self::F32 => f32_at(b, o),
            Self::F64 => f64::from_le_bytes(b[o..o + 8].try_into().unwrap()),
            Self::U8 => b[o] as f64,
            Self::I8 => (b[o] as i8) as f64,
            Self::U16 => u16::from_le_bytes(b[o..o + 2].try_into().unwrap()) as f64,
            Self::I16 => i16::from_le_bytes(b[o..o + 2].try_into().unwrap()) as f64,
            Self::U32 => u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as f64,
            Self::I32 => i32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as f64,
        }
    }
}
pub fn parse_ply(bytes: &[u8]) -> Result<Data> {
    let head = &bytes[..bytes.len().min(65536)];
    let mut offset = 0;
    let mut n = None;
    let mut in_vertex = false;
    let mut seen_vertex = false;
    let mut stride = 0usize;
    let mut props = HashMap::new();
    let mut binary = false;
    let mut end = false;
    for (line_no, raw) in head.split_inclusive(|b| *b == b'\n').enumerate() {
        offset += raw.len();
        let line = std::str::from_utf8(raw)
            .map_err(|_| Error("Invalid PLY header".into()))?
            .trim_end_matches(['\r', '\n']);
        if line_no == 0 && line != "ply" {
            return fail("Not a PLY file");
        }
        let p = line.split_whitespace().collect::<Vec<_>>();
        match p.first().copied() {
            Some("format") => {
                binary = p.get(1) == Some(&"binary_little_endian") && p.get(2) == Some(&"1.0")
            }
            Some("element") => {
                if p.len() != 3 {
                    return fail("Invalid PLY element");
                }
                let c = p[2]
                    .parse::<usize>()
                    .map_err(|_| Error("Invalid PLY element count".into()))?;
                in_vertex = p[1] == "vertex";
                if in_vertex {
                    if seen_vertex {
                        return fail("Duplicate PLY vertex element");
                    }
                    seen_vertex = true;
                    n = Some(c);
                } else if !seen_vertex && c != 0 {
                    return fail("PLY vertex data must precede other non-empty elements");
                }
            }
            Some("property") if in_vertex => {
                if p.len() != 3 || p[1] == "list" {
                    return fail("PLY vertex lists are not supported");
                }
                let t = Ty::parse(p[1])?;
                if props.insert(p[2].to_string(), (stride, t)).is_some() {
                    return fail("Duplicate PLY property");
                }
                stride = stride
                    .checked_add(t.size())
                    .ok_or_else(|| Error("PLY stride overflow".into()))?;
            }
            Some("end_header") => {
                end = true;
                break;
            }
            _ => {}
        }
    }
    if !end || !binary {
        return fail("Only binary little-endian PLY splats are supported");
    }
    let n = n.ok_or_else(|| Error("PLY has no vertex element".into()))?;
    count(n)?;
    let size = n
        .checked_mul(stride)
        .and_then(|x| x.checked_add(offset))
        .ok_or_else(|| Error("PLY size overflow".into()))?;
    if size > bytes.len() {
        return fail("The PLY file is truncated");
    }
    for k in ["x", "y", "z"] {
        if !props.contains_key(k) {
            return fail("The PLY file has no positions");
        }
    }
    let gaussian = [
        "f_dc_0", "f_dc_1", "f_dc_2", "scale_0", "scale_1", "scale_2", "rot_0", "rot_1", "rot_2",
        "rot_3",
    ]
    .iter()
    .all(|k| props.contains_key(*k));
    if !gaussian
        && ["f_dc_0", "scale_0", "rot_0"]
            .iter()
            .any(|k| props.contains_key(*k))
    {
        return fail("Incomplete Gaussian PLY properties");
    }
    let get = |i: usize, k: &str, d: f64| -> Result<f64> {
        match props.get(k) {
            Some((o, t)) => finite(t.read(bytes, offset + i * stride + o)),
            None => Ok(d),
        }
    };
    let mut centers = Vec::with_capacity(n);
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for i in 0..n {
        let p = [get(i, "x", 0.)?, get(i, "y", 0.)?, get(i, "z", 0.)?];
        for a in 0..3 {
            min[a] = min[a].min(p[a]);
            max[a] = max[a].max(p[a]);
        }
        centers.push(p);
    }
    let point_size = 0.25 * length(sub(max, min)) / (n as f64).cbrt();
    let clamp = |v: f64| (v * 255.).round().clamp(0., 255.) as u8;
    let mut splats = Vec::with_capacity(n);
    for (i, p) in centers.into_iter().enumerate() {
        let (cov, rgba) = if gaussian {
            let scale = [
                get(i, "scale_0", 0.)?.exp(),
                get(i, "scale_1", 0.)?.exp(),
                get(i, "scale_2", 0.)?.exp(),
            ];
            let q = [
                get(i, "rot_0", 1.)?,
                get(i, "rot_1", 0.)?,
                get(i, "rot_2", 0.)?,
                get(i, "rot_3", 0.)?,
            ];
            let rgba = [
                clamp(0.5 + 0.28209479177387814 * get(i, "f_dc_0", 0.)?),
                clamp(0.5 + 0.28209479177387814 * get(i, "f_dc_1", 0.)?),
                clamp(0.5 + 0.28209479177387814 * get(i, "f_dc_2", 0.)?),
                clamp(if props.contains_key("opacity") {
                    1. / (1. + (-get(i, "opacity", 0.)?).exp())
                } else {
                    1.
                }),
            ];
            (covariance(scale, q)?, rgba)
        } else {
            (
                covariance([point_size; 3], [1., 0., 0., 0.])?,
                [
                    get(i, "red", 200.)?.clamp(0., 255.) as u8,
                    get(i, "green", 200.)?.clamp(0., 255.) as u8,
                    get(i, "blue", 200.)?.clamp(0., 255.) as u8,
                    255,
                ],
            )
        };
        splats.push(Splat {
            center: p.map(|x| x as f32),
            covariance: cov,
            rgba,
        });
    }
    Ok(Data { splats })
}
impl Data {
    /// Dense-core framing (2nd–98th percentile), unit radius, COLMAP y-down to
    /// editor y-up. The host applies this to the immutable decoded asset.
    pub fn normalization(&self) -> Result<(Vec3, f64)> {
        let b = self.dense_core()?;
        Ok((
            mul(add(b.min, b.max), 0.5),
            (length(sub(b.max, b.min)) / 2.).max(1e-6),
        ))
    }
    pub fn dense_core(&self) -> Result<Bounds> {
        let n = self.splats.len();
        count(n)?;
        let mut lo = [0.; 3];
        let mut hi = [0.; 3];
        for a in 0..3 {
            let mut v = self
                .splats
                .iter()
                .map(|s| s.center[a] as f64)
                .collect::<Vec<_>>();
            for value in &v {
                finite(*value)?;
            }
            v.sort_by(f64::total_cmp);
            lo[a] = v[(n as f64 * 0.02).floor() as usize];
            hi[a] = v[((n as f64 * 0.98).floor() as usize).min(n - 1)];
        }
        Ok(Bounds { min: lo, max: hi })
    }
    /// Stable, exact far-to-near ordering; no 16-bit depth quantization quirk.
    pub fn order(&self, view: [f64; 16]) -> Result<Vec<u32>> {
        if !view.iter().all(|x| x.is_finite()) {
            return fail("Invalid splat view matrix");
        }
        let depth = |s: &Splat| {
            view[2] * s.center[0] as f64
                + view[6] * s.center[1] as f64
                + view[10] * s.center[2] as f64
                + view[14]
        };
        let mut order = (0..self.splats.len() as u32).collect::<Vec<_>>();
        order.sort_by(|a, b| {
            depth(&self.splats[*a as usize]).total_cmp(&depth(&self.splats[*b as usize]))
        });
        Ok(order)
    }
}
