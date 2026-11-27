use super::{
    MAX_TRIANGLES,
    mesh::{self, Mesh},
};
use crate::{Error, Result, fail};

pub fn convert(bytes: &[u8]) -> Result<Vec<u8>> {
    let binary_count = bytes
        .get(80..84)
        .map(|n| u32::from_le_bytes(n.try_into().unwrap()) as usize);
    let mut mesh = Mesh::default();
    if let Some(count) = binary_count
        .filter(|&n| n.checked_mul(50).and_then(|n| n.checked_add(84)) == Some(bytes.len()))
    {
        if count > MAX_TRIANGLES {
            return fail("STL exceeds the triangle limit");
        }
        for facet in bytes[84..].chunks_exact(50) {
            for p in facet[12..48].chunks_exact(12) {
                mesh.positions.push(std::array::from_fn(|i| {
                    f32::from_le_bytes(p[i * 4..i * 4 + 4].try_into().unwrap())
                }));
            }
        }
    } else {
        let source = std::str::from_utf8(bytes)
            .map_err(|_| Error("Invalid or truncated STL file".into()))?;
        let mut lines = source
            .trim_start_matches('\u{feff}')
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty());
        let mut solids = 0;
        while let Some(line) = lines.next() {
            if line.split_whitespace().next() != Some("solid") {
                return fail("Expected an STL solid");
            }
            solids += 1;
            loop {
                let line = lines
                    .next()
                    .ok_or_else(|| Error("Truncated STL solid".into()))?;
                let mut words = line.split_whitespace();
                if words.next() == Some("endsolid") {
                    break;
                }
                let mut words = line.split_whitespace();
                if words.next() != Some("facet")
                    || words.next() != Some("normal")
                    || !mesh::position(&mut words)?.iter().all(|n| n.is_finite())
                    || lines
                        .next()
                        .map(|l| l.split_whitespace().eq(["outer", "loop"]))
                        != Some(true)
                {
                    return fail("Invalid STL facet");
                }
                if mesh.positions.len() / 3 == MAX_TRIANGLES {
                    return fail("STL exceeds the triangle limit");
                }
                for _ in 0..3 {
                    let mut words = lines.next().unwrap_or("").split_whitespace();
                    if words.next() != Some("vertex") {
                        return fail("Invalid STL vertex");
                    }
                    mesh.positions.push(mesh::position(&mut words)?);
                }
                if lines.next() != Some("endloop") || lines.next() != Some("endfacet") {
                    return fail("Invalid STL facet ending");
                }
            }
        }
        if solids == 0 {
            return fail("STL has no solids");
        }
    }
    mesh.indices = (0..mesh.positions.len() as u32).collect();
    mesh::single("STL mesh", mesh)
}
