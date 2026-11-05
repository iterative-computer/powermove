//! Native scene boundary and two-level extraction, ported from Studio's
//! `extract.ts`/`blas.ts`. Evaluators own authored data; this cache owns only
//! derived geometry. Call on a worker, never on the UI thread.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::bvh::{self, WideNode};
use crate::math::{Vec3, v3};
use crate::mesh::Mesh;
use crate::scene::{GpuMaterial, GpuTri, Prepared, Scene};

/// Exact request identity. Time is in the engine's 705,600,000 ticks/second.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneRequest {
    pub composition: String,
    pub revision: u64,
    pub ticks: i64,
    pub width: u32,
    pub height: u32,
}

/// Interface with the scene/core lane. No kernel mutations, JS or DOM calls.
pub trait SceneSource: Send + Sync {
    fn scene(&self, request: &SceneRequest) -> Result<SceneInput, String>;
}

/// Column-major affine object-to-world transform (the editor's matrix order).
#[derive(Clone, Copy, Debug)]
pub struct Transform(pub [f32; 16]);

impl Default for Transform {
    fn default() -> Self {
        Self([
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ])
    }
}

impl Transform {
    fn rows(self) -> Result<[[f32; 4]; 9], String> {
        let w = self.0;
        if !w.iter().all(|x| x.is_finite()) || [w[3], w[7], w[11], w[15]] != [0., 0., 0., 1.] {
            return Err("Studio requires a finite affine transform".into());
        }
        let a = v3(w[0], w[1], w[2]);
        let b = v3(w[4], w[5], w[6]);
        let c = v3(w[8], w[9], w[10]);
        let det = a.dot(b.cross(c));
        if det == 0. || !det.is_finite() {
            return Err("Studio refuses a singular instance transform".into());
        }
        let inv = [b.cross(c) / det, c.cross(a) / det, a.cross(b) / det];
        let t = v3(w[12], w[13], w[14]);
        let out = [
            inv[0].to4(-inv[0].dot(t)),
            inv[1].to4(-inv[1].dot(t)),
            inv[2].to4(-inv[2].dot(t)),
            [inv[0].x, inv[1].x, inv[2].x, 0.],
            [inv[0].y, inv[1].y, inv[2].y, 0.],
            [inv[0].z, inv[1].z, inv[2].z, 0.],
            [w[0], w[4], w[8], 0.],
            [w[1], w[5], w[9], 0.],
            [w[2], w[6], w[10], 0.],
        ];
        if !out.iter().flatten().all(|x| x.is_finite()) {
            return Err("Studio transform inverse overflow".into());
        }
        Ok(out)
    }

    pub fn point(self, p: Vec3) -> Vec3 {
        let w = self.0;
        v3(
            w[0] * p.x + w[4] * p.y + w[8] * p.z + w[12],
            w[1] * p.x + w[5] * p.y + w[9] * p.z + w[13],
            w[2] * p.x + w[6] * p.y + w[10] * p.z + w[14],
        )
    }
}

pub struct MeshInstance {
    pub layer: String,
    /// Geometry is immutable; the same Arc reuses its BLAS after motion.
    pub mesh: Arc<Mesh>,
    pub material: usize,
    pub world: Transform,
}

/// Equal-sized sRGB RGBA8 map-array layers, already oriented to mesh UVs.
pub struct TextureMaps {
    pub width: u32,
    pub height: u32,
    pub layers: Vec<Vec<u8>>,
}

/// Evaluated view values the original Rust prototype did not carry.
#[derive(Clone, Copy, Debug)]
pub struct View {
    /// Forward, right, up in world space, preserving authored camera roll.
    pub basis: Option<[Vec3; 3]>,
    pub ground: Option<Vec3>,
    pub floor_y: f32,
    pub shadows: bool,
}
impl Default for View {
    fn default() -> Self {
        Self {
            basis: None,
            ground: None,
            floor_y: 0.,
            shadows: true,
        }
    }
}

pub struct SceneInput {
    /// Camera, light, environment and backdrop values. Objects are ignored:
    /// geometry comes exclusively from `instances`.
    pub scene: Scene,
    pub instances: Vec<MeshInstance>,
    /// Full physical material records, including map indices and opacity.
    /// None uses the Scene's materials.
    pub materials: Option<Vec<GpuMaterial>>,
    pub maps: Option<TextureMaps>,
    pub view: View,
}

struct Blas {
    mesh: Arc<Mesh>,
    positions: Vec<[f32; 4]>,
    tris: Vec<GpuTri>,
    nodes: Vec<WideNode>,
    bounds: (Vec3, Vec3),
}

#[derive(Default)]
pub struct Extractor {
    cache: HashMap<usize, Blas>,
    pub builds: u64,
}

pub struct Extracted {
    pub(crate) geometry: Vec<Arc<Mesh>>,
    pub(crate) tlas_capacity: usize,
    pub prepared: Prepared,
    /// Stable first-appearance order; indices are exactly the shader's IDs.
    pub layers: Vec<String>,
    pub bounds: (Vec3, Vec3),
}

fn build(mesh: Arc<Mesh>) -> Result<Blas, String> {
    if mesh
        .positions
        .iter()
        .chain(&mesh.normals)
        .any(|p| ![p.x, p.y, p.z].iter().all(|x| x.is_finite()))
        || mesh.uvs.iter().flatten().any(|x| !x.is_finite())
    {
        return Err("non-finite Studio geometry".into());
    }
    let mut corners = Vec::new();
    let mut raw = Vec::new();
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for t in &mesh.indices {
        if t.iter().any(|&i| i as usize >= mesh.positions.len()) {
            return Err("Studio triangle index out of bounds".into());
        }
        let p = t.map(|i| mesh.positions[i as usize]);
        for v in p {
            lo = lo.min(v);
            hi = hi.max(v);
        }
        let face = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
        let uv = t.map(|i| mesh.uvs.get(i as usize).copied().unwrap_or([0.; 2]));
        raw.push(GpuTri {
            n: t.map(|i| {
                mesh.normals
                    .get(i as usize)
                    .copied()
                    .unwrap_or(face)
                    .to4(0.)
            }),
            uv01: [uv[0][0], uv[0][1], uv[1][0], uv[1][1]],
            uv2m: [uv[2][0], uv[2][1], 0., 0.],
            tangent: crate::scene::tangent(p, uv),
        });
        corners.push(p);
    }
    let (nodes, order) = bvh::build(&corners);
    Ok(Blas {
        mesh,
        positions: order
            .iter()
            .flat_map(|&i| corners[i as usize].map(|p| p.to4(0.)))
            .collect(),
        tris: order.iter().map(|&i| raw[i as usize]).collect(),
        nodes: bvh::widen(&nodes),
        bounds: (lo, hi),
    })
}

impl Extractor {
    pub fn extract(&mut self, input: &SceneInput) -> Result<Extracted, String> {
        let mut layers = Vec::new();
        let mut ids = BTreeMap::new();
        let mut used = Vec::new();
        let mut boxes = Vec::new();
        let mut transforms = Vec::new();
        let materials = input
            .materials
            .clone()
            .unwrap_or_else(|| input.scene.gpu_materials());
        let mut positions = Vec::new();
        let mut tris = Vec::new();
        let mut blases = Vec::new();
        let mut roots = Vec::new();
        let mut uploaded = HashMap::new();
        let mut geometry = Vec::new();
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        // TLAS capacity is independent of the SAH tree's shape.
        let capacity = input.instances.len() * 2 + 1;
        for instance in &input.instances {
            if instance.mesh.indices.is_empty() {
                continue;
            }
            if instance.material >= materials.len() || instance.layer.is_empty() {
                return Err("Studio instance has no material or layer ID".into());
            }
            let mut rows = instance.world.rows()?;
            let key = Arc::as_ptr(&instance.mesh) as usize;
            if let std::collections::hash_map::Entry::Vacant(entry) = self.cache.entry(key) {
                entry.insert(build(instance.mesh.clone())?);
                self.builds += 1;
            }
            used.push(key);
            let b = &self.cache[&key];
            debug_assert!(Arc::ptr_eq(&b.mesh, &instance.mesh));
            let next = layers.len() as u32;
            let id = *ids.entry(instance.layer.clone()).or_insert_with(|| {
                layers.push(instance.layer.clone());
                next
            });
            if id >= 16_777_216 || instance.material >= 16_777_216 {
                return Err("Studio float ID capacity exceeded".into());
            }
            let (mut a, mut z) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for x in [b.bounds.0.x, b.bounds.1.x] {
                for y in [b.bounds.0.y, b.bounds.1.y] {
                    for z0 in [b.bounds.0.z, b.bounds.1.z] {
                        let p = instance.world.point(v3(x, y, z0));
                        if ![p.x, p.y, p.z].iter().all(|v| v.is_finite()) {
                            return Err("Studio world bounds overflow".into());
                        }
                        a = a.min(p);
                        z = z.max(p);
                    }
                }
            }
            lo = lo.min(a);
            hi = hi.max(z);
            boxes.push([a, z, a]);
            rows[4][3] = (instance.material + 1) as f32;
            rows[5][3] = (id + 1) as f32;
            transforms.push(rows);
            if let Some(&root) = uploaded.get(&key) {
                roots.push(root);
                continue;
            }
            let tri_offset = tris.len() as u32;
            let node_offset = (capacity + blases.len()) as u32;
            roots.push(node_offset);
            uploaded.insert(key, node_offset);
            geometry.push(instance.mesh.clone());
            positions.extend_from_slice(&b.positions);
            tris.extend_from_slice(&b.tris);
            blases.extend(b.nodes.iter().map(|n| {
                let mut n = *n;
                for (link, count) in [(&mut n.left, n.lcount), (&mut n.right, n.rcount)] {
                    if count != u32::MAX {
                        *link += if count == 0 { node_offset } else { tri_offset };
                    }
                }
                n
            }));
        }
        self.cache.retain(|k, _| used.contains(k));
        let (top, order) = bvh::build(&boxes);
        let mut nodes = bvh::widen(&top);
        nodes.resize(capacity, WideNode::default());
        nodes.extend(blases);
        // Reuse exactly the source's softbox/light encoding.
        let lights = input.scene.gpu_lights();
        let mut sdata: Vec<_> = lights
            .iter()
            .flat_map(|l| [l.pos, l.u, l.v, l.radiance])
            .collect();
        for &i in &order {
            let mut rows = transforms[i as usize];
            rows[3][3] = roots[i as usize] as f32;
            sdata.extend(rows);
        }
        if boxes.is_empty() {
            lo = Vec3::ZERO;
            hi = Vec3::ZERO;
        }
        Ok(Extracted {
            geometry,
            tlas_capacity: capacity,
            prepared: Prepared {
                positions,
                tris,
                nodes,
                materials,
                lights,
                sdata,
            },
            layers,
            bounds: (lo, hi),
        })
    }
}
