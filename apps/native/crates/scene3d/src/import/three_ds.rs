use super::mesh::{Mesh, Scene, srgb};
use crate::{Error, Result, fail};
use std::collections::{BTreeMap, HashMap};

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let (value, rest) = self
            .0
            .split_at_checked(n)
            .ok_or_else(|| Error("Truncated 3DS chunk".into()))?;
        self.0 = rest;
        Ok(value)
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn float(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn name(&mut self) -> Result<String> {
        let end = self
            .0
            .iter()
            .position(|&b| b == 0)
            .filter(|&len| len <= 4096)
            .ok_or_else(|| Error("Invalid 3DS name".into()))?;
        Ok(String::from_utf8_lossy(&self.take(end + 1)?[..end]).into_owned())
    }
    fn chunk(&mut self) -> Result<Option<(u16, Reader<'a>)>> {
        if self.0.is_empty() {
            return Ok(None);
        }
        let tag = self.u16()?;
        let size = u32::from_le_bytes(self.take(4)?.try_into().unwrap()) as usize;
        let body = size
            .checked_sub(6)
            .ok_or_else(|| Error("Invalid 3DS chunk size".into()))?;
        Ok(Some((tag, Reader(self.take(body)?))))
    }
}

pub fn convert(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut file = Reader(bytes);
    let Some((0x4d4d, mut main)) = file.chunk()? else {
        return fail("Invalid 3DS signature");
    };
    let mut scene = Scene::default();
    let mut roots = Vec::new();
    while let Some((tag, mut editor)) = main.chunk()? {
        if tag != 0x3d3d {
            continue;
        }
        let mut materials = HashMap::new();
        let mut objects = Vec::new();
        while let Some((tag, mut entry)) = editor.chunk()? {
            match tag {
                0x4000 => {
                    if objects.len() >= super::mesh::MAX_NODES {
                        return fail("3DS exceeds the object limit");
                    }
                    objects.push(entry);
                }
                0xafff => {
                    let mut name = None;
                    let mut rgba = [1.; 4];
                    while let Some((tag, mut value)) = entry.chunk()? {
                        match tag {
                            0xa000 => name = Some(value.name()?),
                            0xa020 => {
                                let mut linear = false;
                                while let Some((ty, mut color)) = value.chunk()? {
                                    let is_linear = matches!(ty, 0x12 | 0x13);
                                    if linear && !is_linear {
                                        continue;
                                    }
                                    let rgb = match ty {
                                        0x10 | 0x13 => {
                                            [color.float()?, color.float()?, color.float()?]
                                        }
                                        0x11 | 0x12 => {
                                            let c = color.take(3)?;
                                            [c[0], c[1], c[2]].map(|c| f32::from(c) / 255.)
                                        }
                                        _ => continue,
                                    };
                                    if !rgb.iter().all(|c| c.is_finite()) {
                                        return fail("Invalid 3DS material color");
                                    }
                                    rgba[..3].copy_from_slice(&rgb.map(|c| {
                                        let c = c.clamp(0., 1.);
                                        if is_linear { c } else { srgb(c) }
                                    }));
                                    linear = is_linear;
                                }
                            }
                            _ => {}
                        }
                    }
                    if let Some(name) = name {
                        materials.insert(name.clone(), scene.material(&name, rgba));
                    }
                }
                _ => {}
            }
        }
        for mut object in objects {
            let name = object.name()?;
            while let Some((tag, geometry)) = object.chunk()? {
                if tag == 0x4100 {
                    let parts = mesh(geometry, &materials)?;
                    if parts.is_empty() {
                        continue;
                    }
                    let index = scene.mesh(&name, parts)?;
                    roots.push(scene.node(&name, Some(index), None, vec![])?);
                }
            }
        }
    }
    scene.finish(roots)
}

fn mesh(mut data: Reader<'_>, materials: &HashMap<String, usize>) -> Result<Vec<Mesh>> {
    let mut positions = Vec::new();
    let mut uv = Vec::new();
    let mut faces = Vec::new();
    let mut assignments = Vec::new();
    while let Some((tag, mut body)) = data.chunk()? {
        match tag {
            0x4110 => {
                let count = body.u16()?;
                positions = (0..count)
                    .map(|_| {
                        let (x, y, z) = (body.float()?, body.float()?, body.float()?);
                        Ok([x, z, -y])
                    })
                    .collect::<Result<_>>()?;
            }
            0x4120 => {
                let count = body.u16()? as usize;
                faces = (0..count)
                    .map(|_| {
                        let face = [body.u16()?, body.u16()?, body.u16()?];
                        body.take(2)?; // Edge visibility flags.
                        Ok(face)
                    })
                    .collect::<Result<_>>()?;
                assignments = vec![None; count];
                while let Some((tag, mut part)) = body.chunk()? {
                    if tag != 0x4130 {
                        continue;
                    }
                    let material = materials.get(&part.name()?).copied();
                    for _ in 0..part.u16()? {
                        let index = part.u16()? as usize;
                        *assignments
                            .get_mut(index)
                            .ok_or_else(|| Error("Invalid 3DS face index".into()))? = material;
                    }
                }
            }
            0x4140 => {
                let count = body.u16()?;
                uv = (0..count)
                    .map(|_| Ok([body.float()?, 1. - body.float()?]))
                    .collect::<Result<_>>()?;
            }
            _ => {}
        }
    }
    if !uv.is_empty() && uv.len() != positions.len() {
        return fail("Invalid 3DS texture coordinates");
    }
    let mut parts = BTreeMap::<_, (Mesh, HashMap<u16, u32>)>::new();
    for (face, material) in faces.into_iter().zip(assignments) {
        let (part, remap) = parts.entry(material).or_default();
        part.material = material;
        for i in face {
            let p = *positions
                .get(i as usize)
                .ok_or_else(|| Error("Invalid 3DS vertex index".into()))?;
            let index = *remap.entry(i).or_insert_with(|| {
                let index = part.positions.len() as u32;
                part.positions.push(p);
                if !uv.is_empty() {
                    part.uv.push(uv[i as usize]);
                }
                index
            });
            part.indices.push(index);
        }
    }
    Ok(parts.into_values().map(|(part, _)| part).collect())
}
