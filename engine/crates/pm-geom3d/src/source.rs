//! A scene object's geometry source (`sourceSchema` in
//! `core/scene3d/schema.ts`) and the geometry `primitiveGeometry` builds
//! for it.

use crate::primitives::{Geometry, Group, Parameters, Primitive, sequential};

/// A `{ mesh }` source as stored.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MeshSource {
    pub positions: Vec<f64>,
    pub indices: Option<Vec<u32>>,
    pub normals: Option<Vec<f64>>,
    pub uvs: Option<Vec<f64>>,
    pub groups: Vec<Group>,
}

/// A geometry source other than an imported asset.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    Primitive(Primitive),
    /// `{ lathe: [[x, y], …], segments }` (`segments || 48`).
    Lathe {
        points: Vec<[f64; 2]>,
        segments: f64,
    },
    /// `{ extrude: [[x, y], …], depth, bevel }`.
    Extrude {
        points: Vec<[f64; 2]>,
        depth: f64,
        bevel: f64,
    },
    Mesh(MeshSource),
}

impl Source {
    /// `{ primitive, parameters }` with the schema's defaults applied.
    pub fn primitive(name: &str, p: &Parameters) -> Option<Source> {
        Primitive::from_parameters(name, p).map(Source::Primitive)
    }

    /// Writes an identity of the source into `out`: equal bytes for
    /// equal sources (floats by their bits), so a hash of them keys the
    /// geometry. Mesh, lathe, and extrude point lists are written as raw
    /// bits, not formatted.
    pub fn write_key(&self, out: &mut Vec<u8>) {
        fn floats(out: &mut Vec<u8>, v: &[f64]) {
            out.extend((v.len() as u64).to_le_bytes());
            for x in v {
                out.extend(x.to_bits().to_le_bytes());
            }
        }
        fn points(out: &mut Vec<u8>, v: &[[f64; 2]]) {
            out.extend((v.len() as u64).to_le_bytes());
            for [x, y] in v {
                out.extend(x.to_bits().to_le_bytes());
                out.extend(y.to_bits().to_le_bytes());
            }
        }
        fn optional(out: &mut Vec<u8>, v: Option<&[f64]>) {
            match v {
                Some(v) => {
                    out.push(1);
                    floats(out, v);
                }
                None => out.push(0),
            }
        }
        match self {
            Source::Primitive(p) => {
                out.push(0);
                out.extend(format!("{p:?}").as_bytes());
            }
            Source::Lathe {
                points: p,
                segments,
            } => {
                out.push(1);
                points(out, p);
                out.extend(segments.to_bits().to_le_bytes());
            }
            Source::Extrude {
                points: p,
                depth,
                bevel,
            } => {
                out.push(2);
                points(out, p);
                out.extend(depth.to_bits().to_le_bytes());
                out.extend(bevel.to_bits().to_le_bytes());
            }
            Source::Mesh(m) => {
                out.push(3);
                floats(out, &m.positions);
                match &m.indices {
                    Some(i) => {
                        out.push(1);
                        out.extend((i.len() as u64).to_le_bytes());
                        for v in i {
                            out.extend(v.to_le_bytes());
                        }
                    }
                    None => out.push(0),
                }
                optional(out, m.normals.as_deref());
                optional(out, m.uvs.as_deref());
                out.extend((m.groups.len() as u64).to_le_bytes());
                for g in &m.groups {
                    for v in [g.start, g.count, g.material_index] {
                        out.extend(v.to_le_bytes());
                    }
                }
            }
        }
    }

    /// The geometry three.js builds for the source.
    pub fn geometry(&self) -> Geometry {
        match self {
            Source::Primitive(p) => p.geometry(),
            Source::Lathe { points, segments } => {
                let s = if *segments == 0.0 || segments.is_nan() {
                    48.0
                } else {
                    *segments
                };
                crate::shapes::lathe_geometry(points, s)
            }
            Source::Extrude {
                points,
                depth,
                bevel,
            } => crate::shapes::extrude_geometry(points, *depth, *bevel),
            Source::Mesh(m) => mesh_geometry(m),
        }
    }
}

/// `{ mesh }`: the stored attributes as `Float32BufferAttribute`s, the
/// index when present, normals computed when absent, and the groups.
pub fn mesh_geometry(m: &MeshSource) -> Geometry {
    let positions: Vec<f32> = m.positions.iter().map(|v| *v as f32).collect();
    let count = positions.len() / 3;
    let normals = match &m.normals {
        Some(n) => n.iter().map(|v| *v as f32).collect(),
        None => crate::normals::compute_vertex_normals(&positions, m.indices.as_deref()),
    };
    let uvs = m
        .uvs
        .as_ref()
        .map(|u| u.iter().map(|v| *v as f32).collect())
        .unwrap_or_default();
    Geometry {
        positions,
        normals,
        uvs,
        indices: m.indices.clone().unwrap_or_else(|| sequential(count)),
        indexed: m.indices.is_some(),
        groups: m.groups.clone(),
    }
}
