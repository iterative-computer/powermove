use super::{
    MAX_TRIANGLES,
    mesh::{self, MAX_FACE_VERTICES, MAX_VERTICES, Mesh, number},
};
use crate::{Error, Result, fail};

pub fn convert(bytes: &[u8]) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(bytes).map_err(|_| Error("OFF must be text".into()))?;
    let mut lines = text
        .trim_start_matches('\u{feff}')
        .lines()
        .map(|l| l.split('#').next().unwrap().trim())
        .filter(|l| !l.is_empty());
    let mut header = lines.next().unwrap_or("").split_whitespace();
    let kind = header.next().unwrap_or("");
    if !matches!(kind, "OFF" | "COFF" | "NOFF" | "CNOFF" | "STOFF") {
        return fail("Unsupported OFF header");
    }
    let mut counts = header.chain(lines.by_ref().take(1).flat_map(str::split_whitespace));
    let vertices: usize = number(counts.next())?;
    let faces: usize = number(counts.next())?;
    let _: usize = number(counts.next())?;
    if vertices > MAX_VERTICES || faces > MAX_TRIANGLES {
        return fail("OFF mesh exceeds import limits");
    }
    let mut mesh = Mesh::default();
    for _ in 0..vertices {
        let mut row = lines.next().unwrap_or("").split_whitespace();
        mesh.positions.push(mesh::position(&mut row)?);
        if matches!(kind, "NOFF" | "CNOFF") {
            mesh.normals.push(mesh::position(&mut row)?);
        }
        if matches!(kind, "COFF" | "CNOFF") {
            let [r, g, b] = mesh::position(&mut row)?;
            let a: Option<f32> = row.next().map(|v| number(Some(v))).transpose()?;
            let scale = if [r, g, b, a.unwrap_or(1.)].into_iter().any(|c| c > 1.) {
                255.
            } else {
                1.
            };
            mesh.colors.push([
                mesh::srgb((r / scale).clamp(0., 1.)),
                mesh::srgb((g / scale).clamp(0., 1.)),
                mesh::srgb((b / scale).clamp(0., 1.)),
                (a.unwrap_or(scale) / scale).clamp(0., 1.),
            ]);
        }
        if kind == "STOFF" {
            mesh.uv.push([number(row.next())?, number(row.next())?]);
        }
    }
    for _ in 0..faces {
        let mut row = lines.next().unwrap_or("").split_whitespace();
        let count: usize = number(row.next())?;
        if !(3..=MAX_FACE_VERTICES).contains(&count) {
            return fail("Invalid OFF face size");
        }
        let face = (0..count)
            .map(|_| number(row.next()))
            .collect::<Result<Vec<_>>>()?;
        mesh::polygon(&mesh.positions, &face, &mut mesh.indices)?;
    }
    mesh::single("OFF mesh", mesh)
}
