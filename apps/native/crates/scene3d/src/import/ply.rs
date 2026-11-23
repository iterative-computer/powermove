use super::{
    MAX_TRIANGLES,
    mesh::{self, MAX_FACE_VERTICES, MAX_VERTICES, Mesh, number},
    splats::Ty,
};
use crate::{Error, Result, fail};

#[derive(Clone, Copy)]
enum Encoding {
    Ascii,
    Little,
    Big,
}

enum Property<'a> {
    Scalar(Ty, &'a str),
    List(Ty, Ty, &'a str),
}

struct Element<'a> {
    name: &'a str,
    count: usize,
    properties: Vec<Property<'a>>,
}

enum Body<'a> {
    Ascii(std::str::SplitWhitespace<'a>),
    Binary { bytes: &'a [u8], big: bool },
}

impl Body<'_> {
    fn read(&mut self, ty: Ty) -> Result<f64> {
        match self {
            Self::Ascii(words) => match ty {
                Ty::F32 | Ty::F64 => number(words.next()),
                Ty::U8 => number::<u8>(words.next()).map(f64::from),
                Ty::I8 => number::<i8>(words.next()).map(f64::from),
                Ty::U16 => number::<u16>(words.next()).map(f64::from),
                Ty::I16 => number::<i16>(words.next()).map(f64::from),
                Ty::U32 => number::<u32>(words.next()).map(f64::from),
                Ty::I32 => number::<i32>(words.next()).map(f64::from),
            },
            Self::Binary { bytes, big } => {
                let (field, rest) = bytes
                    .split_at_checked(ty.size())
                    .ok_or_else(|| Error("Truncated binary PLY".into()))?;
                *bytes = rest;
                if *big {
                    let mut little = [0; 8];
                    little[..field.len()].copy_from_slice(field);
                    little[..field.len()].reverse();
                    Ok(ty.read(&little, 0))
                } else {
                    Ok(ty.read(field, 0))
                }
            }
        }
    }
}

fn integer(s: &str) -> Result<Ty> {
    let ty = Ty::parse(s)?;
    if matches!(ty, Ty::F32 | Ty::F64) {
        return fail("PLY list counts and face indices must be integers");
    }
    Ok(ty)
}

fn color(value: f64, ty: Ty) -> f32 {
    let max = match ty {
        Ty::U8 => u8::MAX as f64,
        Ty::I8 => i8::MAX as f64,
        Ty::U16 => u16::MAX as f64,
        Ty::I16 => i16::MAX as f64,
        Ty::U32 => u32::MAX as f64,
        Ty::I32 => i32::MAX as f64,
        _ => 1.,
    };
    (value / max).clamp(0., 1.) as f32
}

pub fn convert(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut encoding = None;
    let mut elements: Vec<Element<'_>> = Vec::new();
    let mut offset = 0;
    let mut ended = false;
    for (i, raw) in bytes[..bytes.len().min(65536)]
        .split_inclusive(|&b| b == b'\n')
        .enumerate()
    {
        offset += raw.len();
        let line = std::str::from_utf8(raw).map_err(|_| Error("Invalid PLY header".into()))?;
        let words: Vec<_> = line.split_whitespace().collect();
        match words.as_slice() {
            ["ply"] if i == 0 => {}
            ["format", name, "1.0"] if encoding.is_none() => {
                encoding = Some(match *name {
                    "ascii" => Encoding::Ascii,
                    "binary_little_endian" => Encoding::Little,
                    "binary_big_endian" => Encoding::Big,
                    _ => return fail("Unsupported PLY encoding"),
                });
            }
            ["element", name, count] => {
                let count = number(Some(count))?;
                if count > MAX_VERTICES
                    || (*name == "face" && count > MAX_TRIANGLES)
                    || elements.iter().any(|e| e.name == *name)
                {
                    return fail("Invalid or oversized PLY element");
                }
                elements.push(Element {
                    name,
                    count,
                    properties: vec![],
                });
            }
            ["property", ..] => {
                let elem = elements
                    .last_mut()
                    .ok_or_else(|| Error("PLY property has no element".into()))?;
                let prop = match words.as_slice() {
                    [_, ty, name] => Property::Scalar(Ty::parse(ty)?, name),
                    [_, "list", count, ty, name] => {
                        Property::List(integer(count)?, Ty::parse(ty)?, name)
                    }
                    _ => return fail("Invalid PLY property"),
                };
                elem.properties.push(prop);
            }
            ["end_header"] => {
                ended = true;
                break;
            }
            ["comment" | "obj_info", ..] | [] => {}
            _ => return fail("Invalid PLY header"),
        }
    }
    if !ended || !bytes.starts_with(b"ply") {
        return fail("Incomplete PLY header");
    }
    let bytes = &bytes[offset..];
    let mut body = match encoding {
        Some(Encoding::Ascii) => Body::Ascii(
            std::str::from_utf8(bytes)
                .map_err(|_| Error("Invalid ASCII PLY".into()))?
                .split_whitespace(),
        ),
        Some(encoding) => Body::Binary {
            bytes,
            big: matches!(encoding, Encoding::Big),
        },
        None => return fail("PLY has no encoding"),
    };
    let mut mesh = Mesh::default();
    let mut faces = Vec::new();
    let mut face_index_count = 0;
    for element in elements {
        let vertex = element.name == "vertex";
        let face = element.name == "face";
        let has = |name| {
            element
                .properties
                .iter()
                .any(|p| matches!(p, Property::Scalar(_, n) if *n == name))
        };
        let normals = vertex && ["nx", "ny", "nz"].into_iter().all(has);
        let colors = vertex && ["red", "green", "blue"].into_iter().all(has);
        if (vertex && !["x", "y", "z"].into_iter().all(has))
            || (element.count > 0 && element.properties.is_empty())
        {
            return fail("Missing PLY element properties");
        }
        for _ in 0..element.count {
            let mut position = [0.; 3];
            let mut normal = [0.; 3];
            let mut rgba = [1.; 4];
            let mut polygon = None;
            for prop in &element.properties {
                match *prop {
                    Property::Scalar(ty, name) => {
                        let value = body.read(ty)?;
                        if vertex {
                            match name {
                                "x" => position[0] = value as f32,
                                "y" => position[1] = value as f32,
                                "z" => position[2] = value as f32,
                                "nx" => normal[0] = value as f32,
                                "ny" => normal[1] = value as f32,
                                "nz" => normal[2] = value as f32,
                                "red" => rgba[0] = mesh::srgb(color(value, ty)),
                                "green" => rgba[1] = mesh::srgb(color(value, ty)),
                                "blue" => rgba[2] = mesh::srgb(color(value, ty)),
                                "alpha" => rgba[3] = color(value, ty),
                                _ => {}
                            }
                        }
                    }
                    Property::List(count_ty, ty, name) => {
                        let count = body.read(count_ty)?;
                        let indices = face && matches!(name, "vertex_indices" | "vertex_index");
                        if !(0. ..=MAX_VERTICES as f64).contains(&count) {
                            return fail("Invalid PLY list length");
                        }
                        let count = count as usize;
                        if indices {
                            if polygon.is_some()
                                || !(3..=MAX_FACE_VERTICES).contains(&count)
                                || matches!(ty, Ty::F32 | Ty::F64)
                            {
                                return fail("Invalid PLY face");
                            }
                            face_index_count += count - 2;
                            if face_index_count > MAX_TRIANGLES {
                                return fail("PLY exceeds the triangle limit");
                            }
                            let mut indices = Vec::with_capacity(count);
                            for _ in 0..count {
                                let index = body.read(ty)?;
                                if !(0. ..=u32::MAX as f64).contains(&index) {
                                    return fail("Invalid PLY face index");
                                }
                                indices.push(index as u32);
                            }
                            polygon = Some(indices);
                        } else {
                            for _ in 0..count {
                                body.read(ty)?;
                            }
                        }
                    }
                }
            }
            if vertex {
                mesh.positions.push(position);
                if normals {
                    mesh.normals.push(normal);
                }
                if colors {
                    mesh.colors.push(rgba);
                }
            }
            if face {
                faces.push(polygon.ok_or_else(|| Error("PLY face has no vertex indices".into()))?);
            }
        }
    }
    for face in faces {
        mesh::polygon(&mesh.positions, &face, &mut mesh.indices)?;
    }
    mesh::single("PLY mesh", mesh)
}
