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
    Lathe { points: Vec<[f64; 2]>, segments: f64 },
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
