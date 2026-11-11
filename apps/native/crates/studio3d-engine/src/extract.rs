//! Native scene boundary and two-level extraction, ported from Studio's
//! `extract.ts`/`blas.ts`. Evaluators own authored data; this cache owns only
//! derived geometry. Call on a worker, never on the UI thread.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::SceneExt;
use crate::bvh::{self, WideNode};
use crate::math::{Vec3, v3};
use crate::mesh::Mesh;
use crate::scene::{GpuMaterial, GpuTri, Prepared, Scene};

pub use pm_render::scene3d::provider::SceneRequest;

/// Interface with the scene/core lane. No kernel mutations, JS or DOM calls.
pub trait SceneSource: Send + Sync {
    fn scene(&self, request: &SceneRequest) -> Result<SceneInput, String>;
    /// Failed contributions omitted from this immutable scene, by timeline ID.
    fn layer_failures(&self, _request: &SceneRequest) -> Result<Vec<pm_render::LayerFailure>, String> { Ok(vec![]) }
    /// Optional saved/export policy. Legacy sources retain their authored sample
    /// count; native policy is resolved on the worker from the frozen document.
    fn final_quality(&self, _request: &SceneRequest) -> Result<Option<FinalQuality>, String> {
        Ok(None)
    }
    fn auxiliary_layers(&self, _request: &SceneRequest) -> Result<Vec<AuxiliaryLayer>, String> {
        Ok(vec![])
    }
}

/// Non-surface timeline layers still need a compositor answer. World owns the
/// visible environment; fog volumes affect transport without drawing a proxy.
#[derive(Clone, Debug)]
pub struct AuxiliaryLayer {
    pub id: String,
    pub include_void: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FinalQuality {
    pub min_samples: u32,
    pub max_samples: u32,
    pub noise_threshold: f32,
    pub max_bounces: u32,
    pub caustics: bool,
    pub denoise: bool,
    pub half_resolution_guides: bool,
}
impl FinalQuality {
    pub fn validate(self) -> Result<Self, String> {
        if self.min_samples == 0
            || self.min_samples > self.max_samples
            || self.max_samples > 4096
            || !(1..=32).contains(&self.max_bounces)
            || !self.noise_threshold.is_finite()
            || !(0.0..=1.0).contains(&self.noise_threshold)
        {
            return Err("Invalid Studio render quality".into());
        }
        Ok(self)
    }
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
    pub fn validate(self) -> Result<(),String> { self.rows().map(|_|()) }
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

/// Immutable imported vertex data carried beside the shared engine mesh.
#[derive(Clone, Debug, Default)]
pub struct VertexAttributes {
    pub colors: Arc<Vec<[f32; 4]>>,
    pub secondary_uvs: Arc<Vec<[f32; 2]>>,
    pub flat_shading: bool,
}
impl VertexAttributes {
    /// Exact-byte interning preserves BLAS identity across snapshot replacement.
    pub fn shared(self) -> Result<Arc<Self>, String> {
        static CACHE: std::sync::OnceLock<
            std::sync::Mutex<HashMap<Vec<u8>, std::sync::Weak<VertexAttributes>>>,
        > = std::sync::OnceLock::new();
        let mut key = vec![u8::from(self.flat_shading)];
        key.extend_from_slice(&(self.colors.len() as u64).to_le_bytes());
        key.extend_from_slice(bytemuck::cast_slice(&self.colors));
        key.extend_from_slice(bytemuck::cast_slice(&self.secondary_uvs));
        let mut cache = CACHE
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| "Studio vertex attribute cache poisoned")?;
        if let Some(value) = cache.get(&key).and_then(std::sync::Weak::upgrade) {
            return Ok(value);
        }
        cache.retain(|_, value| value.strong_count() > 0);
        if cache.len() >= 512 {
            cache.clear();
        }
        let value = Arc::new(self);
        cache.insert(key, Arc::downgrade(&value));
        Ok(value)
    }
}

#[derive(Clone)]
pub struct MeshInstance {
    pub layer: String,
    /// Geometry is immutable; the same Arc reuses its BLAS after motion.
    pub mesh: Arc<Mesh>,
    pub attributes: Option<Arc<VertexAttributes>>,
    pub material: usize,
    /// Local face-material IDs map through this palette. None retains the
    /// engine's absolute scene-material IDs; empty face data uses `material`.
    pub material_palette: Option<Arc<Vec<usize>>>,
    /// Shadow-only objects remain in transport while primary camera rays skip them.
    pub primary_visible: bool,
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
    pub ambient: Vec3,
    /// Half the vertical extent in scene units; None selects perspective.
    pub orthographic_span: Option<f32>,
    pub clip_planes: Option<[f32; 2]>,
    /// Blades, rotation radians, roundness, aspect; matches shared shader data.
    pub iris: [f32; 4],
}
impl Default for View {
    fn default() -> Self {
        Self {
            basis: None,
            ground: None,
            floor_y: 0.,
            shadows: true,
            ambient: Vec3::ZERO,
            orthographic_span: None,
            clip_planes: None,
            iris: [0., 0., 0., 1.],
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
    _attributes: Option<Arc<VertexAttributes>>,
    positions: Vec<[f32; 4]>,
    tris: Vec<GpuTri>,
    nodes: Vec<WideNode>,
    bounds: (Vec3, Vec3),
}

#[derive(Default)]
pub struct Extractor {
    cache: HashMap<(usize, usize), Blas>,
    world_cache: Option<(Vec<u8>, Vec<[f32; 4]>)>,
    pub builds: u64,
}

pub struct Extracted {
    pub(crate) tlas_capacity: usize,
    pub prepared: Prepared,
    /// Stable first-appearance order; indices are exactly the shader's IDs.
    pub layers: Vec<String>,
    pub bounds: (Vec3, Vec3),
}

pub fn validate_mesh(mesh: &Mesh, extra: Option<&VertexAttributes>) -> Result<(), String> {
    if extra.is_some_and(|a| {
        (!a.colors.is_empty() && a.colors.len() != mesh.positions.len())
            || (!a.secondary_uvs.is_empty() && a.secondary_uvs.len() != mesh.positions.len())
            || a.colors
                .iter()
                .flatten()
                .chain(a.secondary_uvs.iter().flatten())
                .any(|v| !v.is_finite())
    }) {
        return Err("Invalid Studio imported vertex attributes".into());
    }
    if mesh
        .positions
        .iter()
        .chain(&mesh.normals)
        .chain(&mesh.colors)
        .any(|p| ![p.x, p.y, p.z].iter().all(|x| x.is_finite()))
        || mesh.uvs.iter().flatten().any(|x| !x.is_finite())
    {
        return Err("non-finite Studio geometry".into());
    }
    if (!mesh.colors.is_empty() && mesh.colors.len() != mesh.positions.len())
        || (!mesh.triangle_materials.is_empty()
            && mesh.triangle_materials.len() != mesh.indices.len())
        || mesh.triangle_materials.iter().any(|&m| m >= 16_777_216)
    {
        return Err("Invalid Studio vertex colors or face materials".into());
    }
    if mesh.indices.iter().flatten().any(|&i|i as usize>=mesh.positions.len()) {
        return Err("Studio triangle index out of bounds".into());
    }
    Ok(())
}

fn build(mesh: Arc<Mesh>, attributes: Option<Arc<VertexAttributes>>) -> Result<Blas, String> {
    let extra=attributes.as_deref();
    validate_mesh(&mesh,extra)?;
    let mut corners = Vec::new();
    let mut raw = Vec::new();
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for (face_index, t) in mesh.indices.iter().enumerate() {
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
        let secondary = t.map(|i| {
            extra
                .and_then(|a| a.secondary_uvs.get(i as usize))
                .copied()
                .unwrap_or_else(|| mesh.uvs.get(i as usize).copied().unwrap_or([0.; 2]))
        });
        raw.push(GpuTri {
            n: t.map(|i| {
                let normal = if extra.is_some_and(|a| a.flat_shading) {
                    face
                } else {
                    mesh.normals.get(i as usize).copied().unwrap_or(face)
                };
                let color = extra
                    .and_then(|a| a.colors.get(i as usize))
                    .copied()
                    .unwrap_or_else(|| mesh.colors.get(i as usize).map_or([1.; 4], |c| c.to4(1.)));
                let b = color.map(|v| (v.clamp(0., 1.) * 255.).round() as u32);
                normal.to4(f32::from_bits(b[0] | b[1] << 8 | b[2] << 16 | b[3] << 24))
            }),
            uv01: [uv[0][0], uv[0][1], uv[1][0], uv[1][1]],
            uv2m: [
                uv[2][0],
                uv[2][1],
                mesh.triangle_materials
                    .get(face_index)
                    .copied()
                    .unwrap_or(0) as f32,
                0.,
            ],
            tangent: crate::packing::tangent(p, uv),
            uv01_secondary: [
                secondary[0][0],
                secondary[0][1],
                secondary[1][0],
                secondary[1][1],
            ],
            uv2_secondary: [secondary[2][0], secondary[2][1], 0., 0.],
            tangent_secondary: crate::packing::tangent(p, secondary),
        });
        corners.push(p);
    }
    let (nodes, order) = bvh::build(&corners);
    Ok(Blas {
        mesh,
        _attributes: attributes,
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
        let mut materials = input
            .materials
            .clone()
            .unwrap_or_else(|| input.scene.gpu_materials());
        let mut positions = Vec::new();
        let mut tris = Vec::new();
        let mut blases = Vec::new();
        let mut roots = Vec::new();
        let mut uploaded = HashMap::new();
        let mut geometries = Vec::new();
        let mut placement_inputs = Vec::new();
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
            let binding = (
                (
                    Arc::as_ptr(&instance.mesh) as usize,
                    instance
                        .attributes
                        .as_ref()
                        .map_or(0, |a| Arc::as_ptr(a) as usize),
                ),
                instance.material,
                instance.material_palette.clone(),
            );
            if !uploaded.contains_key(&binding) {
                let face_material = |local: usize| -> Result<usize, String> {
                    let m = if instance.mesh.triangle_materials.is_empty() {
                        instance.material
                    } else if let Some(palette) = &instance.material_palette {
                        *palette
                            .get(local)
                            .ok_or("Studio face material has no palette entry")?
                    } else {
                        local
                    };
                    if m >= materials.len() || m >= 16_777_216 {
                        return Err("Studio face material out of bounds".into());
                    }
                    Ok(m)
                };
                let slots = instance
                    .mesh
                    .triangle_materials
                    .iter()
                    .copied()
                    .chain(
                        std::iter::once(0).filter(|_| instance.mesh.triangle_materials.is_empty()),
                    )
                    .map(face_material)
                    .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
                if !instance.mesh.colors.is_empty()
                    || instance
                        .attributes
                        .as_ref()
                        .is_some_and(|a| !a.colors.is_empty())
                {
                    for slot in slots {
                        materials[slot].policy[1] = (materials[slot].policy[1] as u32 | 16) as f32;
                    }
                }
            }
            let mut rows = instance.world.rows()?;
            let key = binding.0;
            if let std::collections::hash_map::Entry::Vacant(entry) = self.cache.entry(key) {
                entry.insert(build(instance.mesh.clone(), instance.attributes.clone())?);
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
            if id >= 8_388_607 || instance.material >= 16_777_216 {
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
            rows[4][3] = ((id + 1) * 2) as f32;
            rows[5][3] = f32::from(!instance.primary_visible);
            transforms.push(rows);
            if let Some(&(root, geometry_index, triangle_offset)) = uploaded.get(&binding) {
                placement_inputs.push((geometry_index, triangle_offset, instance.world));
                roots.push(root);
                continue;
            }
            let tri_offset = tris.len() as u32;
            let node_offset = (capacity + blases.len()) as u32;
            roots.push(node_offset);
            let geometry_index = geometries.len();
            geometries.push(crate::gpu::RtGeometry {
                first_vertex: tri_offset * 3,
                vertex_count: b.positions.len() as u32,
            });
            placement_inputs.push((geometry_index, tri_offset, instance.world));
            uploaded.insert(binding, (node_offset, geometry_index, tri_offset));
            positions.extend_from_slice(&b.positions);
            tris.extend(b.tris.iter().map(|t| {
                let mut t = *t;
                t.uv2m[2] = if instance.mesh.triangle_materials.is_empty() {
                    instance.material
                } else if let Some(palette) = &instance.material_palette {
                    palette[t.uv2m[2] as usize]
                } else {
                    t.uv2m[2] as usize
                } as f32;
                t
            }));
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
        let mut placements = Vec::new();
        for &i in &order {
            let (geometry, triangle_offset, transform) = placement_inputs[i as usize];
            let w = transform.0;
            placements.push(crate::gpu::RtPlacement {
                geometry,
                triangle_offset,
                transform: [
                    w[0], w[4], w[8], w[12], w[1], w[5], w[9], w[13], w[2], w[6], w[10], w[14],
                ],
            });
            let mut rows = transforms[i as usize];
            rows[3][3] = roots[i as usize] as f32;
            sdata.extend(rows);
        }
        // The physical sky bake is independent of camera and object motion.
        // Retain its trailer until the exact world/light input changes.
        let world = &input.scene.world;
        let mut key = format!(
            "{:?}{:?}{:?}{:?}{lights:?}",
            world.sky, world.atmosphere, world.volumes, world.adaptive
        )
        .into_bytes();
        if let Some(map) = &world.hdri {
            key.extend_from_slice(
                format!(
                    "map:{:?}",
                    (
                        map.width,
                        map.height,
                        map.rotation,
                        map.intensity,
                        map.visible
                    )
                )
                .as_bytes(),
            );
            key.extend_from_slice(bytemuck::cast_slice(&map.pixels));
        }
        if self.world_cache.as_ref().is_none_or(|(old, _)| *old != key) {
            self.world_cache = Some((key, world.pack(&lights)));
        }
        sdata.extend_from_slice(&self.world_cache.as_ref().unwrap().1);
        if boxes.is_empty() {
            lo = Vec3::ZERO;
            hi = Vec3::ZERO;
        }
        Ok(Extracted {
            tlas_capacity: capacity,
            prepared: Prepared {
                instancing: Some(pm_studio::geometry::Instancing {
                    geometries,
                    placements,
                    top_nodes: capacity,
                }),
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
