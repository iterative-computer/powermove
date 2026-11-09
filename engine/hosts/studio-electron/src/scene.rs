//! Scene contract: pm_studio::scene::Scene plus binary mesh attachments and
//! column-major transforms. Geometry/BVH/Metal BLAS are reused while dragging.
use crate::protocol::Range;
use glam::{Mat3, Mat4, Vec3 as GVec3};
use pm_studio::{
    bvh,
    gpu::{RtGeometry, RtPlacement},
    math::{Vec3, v3},
    scene::{Prepared, Scene},
};
use serde::{Deserialize, Serialize};

#[cfg(test)]
pub const IDENTITY: [f32; 16] = [
    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
];
#[derive(Debug, Deserialize)]
pub struct MeshBuffers {
    pub object: usize,
    pub positions: Range,
    pub normals: Range,
    pub uvs: Range,
    pub indices: Range,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Transform {
    pub object: usize,
    pub matrix: [f32; 16],
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Identity {
    pub layer: String,
    pub part: String,
}

pub struct ResidentScene {
    pub scene: Scene,
    pub identities: Vec<Identity>,
    pub transforms: Vec<Mat4>,
    pub prepared: Prepared,
    pub geometries: Vec<RtGeometry>,
    pub placements: Vec<RtPlacement>,
    pub top_nodes: usize,
    local_bounds: Vec<(Vec3, Vec3)>,
    roots: Vec<u32>,
    pub object_indices: Vec<usize>,
    instance_layers: Vec<Option<u32>>,
    world_data: Vec<[f32;4]>,
}

fn point(m: Mat4, p: Vec3) -> Vec3 {
    let p = m.transform_point3(GVec3::new(p.x, p.y, p.z));
    v3(p.x, p.y, p.z)
}
fn matrix(t: &[f32; 16]) -> Result<Mat4, String> {
    let m = Mat4::from_cols_array(t);
    if !m.is_finite()
        || m.determinant().abs() < 1e-12
        || t[3] != 0.
        || t[7] != 0.
        || t[11] != 0.
        || t[15] != 1.
    {
        return Err("transform must be a finite, invertible affine matrix".into());
    }
    Ok(m)
}

impl ResidentScene {
    pub fn new(
        mut scene: Scene,
        buffers: Vec<MeshBuffers>,
        binary: &[u8],
        identities: Vec<Identity>,
    ) -> Result<Self, String> {
        for b in buffers {
            let o = scene
                .objects
                .get_mut(b.object)
                .ok_or("mesh object index out of range")?;
            o.mesh.positions = b
                .positions
                .typed::<[f32; 3]>(binary)?
                .into_iter()
                .map(|p| v3(p[0], p[1], p[2]))
                .collect();
            o.mesh.normals = b
                .normals
                .typed::<[f32; 3]>(binary)?
                .into_iter()
                .map(|p| v3(p[0], p[1], p[2]))
                .collect();
            o.mesh.uvs = b.uvs.typed(binary)?;
            o.mesh.indices = b.indices.typed(binary)?;
        }
        validate(&scene)?;
        let source_objects=scene.objects.len();
        scene=scene.expanded()?;
        validate(&scene)?;
        let mut identities=identities;
        if !identities.is_empty() && identities.len()==source_objects {for i in source_objects..scene.objects.len(){identities.push(Identity{layer:i.to_string(),part:format!("terrain-{i}")});}}
        let identities = if identities.is_empty() {
            (0..scene.objects.len())
                .map(|i| Identity {
                    layer: scene
                        .layers
                        .as_ref()
                        .and_then(|l| l.get(i))
                        .copied()
                        .unwrap_or(i as u32)
                        .to_string(),
                    part: i.to_string(),
                })
                .collect()
        } else {
            identities
        };
        if identities.len() != scene.objects.len() {
            return Err("identities must match object count".into());
        }
        // One local-space BLAS per object, with stable offsets.
        let object_indices:Vec<usize>=if scene.instances.is_empty(){(0..scene.objects.len()).collect()}else{scene.instances.iter().map(|i|i.object).collect()};
        let transforms=if scene.instances.is_empty(){vec![Mat4::IDENTITY;scene.objects.len()]}else{scene.instances.iter().map(pm_studio::geometry::matrix).collect()};
        let instance_layers=if scene.instances.is_empty(){vec![None;scene.objects.len()]}else{scene.instances.iter().map(|i|i.layer).collect()};
        let top_nodes = object_indices.len().max(1) + 1;
        let mut prepared = scene.clone_without_objects().prepare();
        for o in &scene.objects {if !o.mesh.colors.is_empty() {
            prepared.materials[o.material].policy[1]=(prepared.materials[o.material].policy[1] as u32|16) as f32;
            for &m in &o.mesh.triangle_materials {prepared.materials[m].policy[1]=(prepared.materials[m].policy[1] as u32|16) as f32;}
        }}
        prepared.positions.clear();
        prepared.tris.clear();
        prepared.nodes = vec![bvh::WideNode::default(); top_nodes];
        let mut geometries = Vec::new();
        let mut roots = Vec::new();
        let mut local_bounds = Vec::new();
        for (i, o) in scene.objects.iter().enumerate() {
            let one = Scene {
                world: Default::default(),
                objects: vec![o.clone()],
                layers: Some(vec![
                    scene
                        .layers
                        .as_ref()
                        .and_then(|l| l.get(i))
                        .copied()
                        .unwrap_or(i as u32),
                ]),
                ..scene.clone_without_objects()
            };
            let p = one.prepare();
            let tri_offset = prepared.tris.len() as u32;
            let node_offset = prepared.nodes.len() as u32;
            roots.push(node_offset);
            geometries.push(RtGeometry {
                first_vertex: tri_offset * 3,
                vertex_count: p.positions.len() as u32,
            });
            prepared.positions.extend(p.positions);
            prepared.tris.extend(p.tris);
            for mut n in p.nodes.into_iter().skip(1) {
                for (child, count) in [(&mut n.left, n.lcount), (&mut n.right, n.rcount)] {
                    if count == u32::MAX {
                        continue;
                    }
                    *child = if count == 0 {
                        *child - 1 + node_offset
                    } else {
                        *child + tri_offset
                    };
                }
                prepared.nodes.push(n);
            }
            local_bounds.push(o.mesh.bounds());
        }
        let world_data=scene.world.pack(&prepared.lights);
        let mut r = Self {
            transforms,
            object_indices,
            instance_layers,
            world_data,
            scene,
            identities,
            prepared,
            geometries,
            placements: Vec::new(),
            top_nodes,
            local_bounds,
            roots: roots.clone(),
        };
        r.rebuild_top(&roots);
        Ok(r)
    }

    pub fn patch_transforms(&mut self, edits: &[Transform]) -> Result<(), String> {
        if edits.is_empty() {
            return Ok(());
        }
        // Validate the whole patch before changing any state.
        let edits: Vec<_> = edits
            .iter()
            .map(|e| {
                if e.object >= self.transforms.len() {
                    return Err("transform object index out of range".into());
                }
                Ok((e.object, matrix(&e.matrix)?))
            })
            .collect::<Result<_, String>>()?;
        for (i, m) in edits {
            self.transforms[i] = m;
        }
        self.rebuild_top(&self.roots.clone());
        Ok(())
    }
    fn rebuild_top(&mut self, roots: &[u32]) {
        let boxes: Vec<_> = (0..self.transforms.len())
            .map(|i| {
                let (lo, hi) = self.object_bounds(i);
                [lo, hi, lo]
            })
            .collect();
        let (nodes, order) = bvh::build(&boxes);
        let top = bvh::widen(&nodes);
        self.prepared.nodes[..self.top_nodes].fill(bvh::WideNode::default());
        self.prepared.nodes[..top.len()].copy_from_slice(&top);
        self.prepared.sdata.truncate(self.prepared.lights.len() * 4);
        self.placements.clear();
        for i in order.into_iter().map(|i| i as usize) {
            let source=self.object_indices[i];
            let m = self.transforms[i];
            let inv = m.inverse();
            let n = Mat3::from_mat4(inv.transpose());
            let iv = inv.to_cols_array_2d();
            let mw = m.to_cols_array_2d();
            let nr = n.to_cols_array_2d();
            for r in 0..3 {
                self.prepared
                    .sdata
                    .push([iv[0][r], iv[1][r], iv[2][r], iv[3][r]]);
            }
            for r in 0..3 {
                self.prepared.sdata.push([
                    nr[0][r],
                    nr[1][r],
                    nr[2][r],
                    match r {0=>roots[source] as f32,1=>self.instance_layers[i].map_or(0.,|l|((l+1)*2) as f32),_=>0.},
                ]);
            }
            for r in 0..3 {
                self.prepared.sdata.push([mw[0][r], mw[1][r], mw[2][r], 0.]);
            }
            self.placements.push(RtPlacement {
                geometry: source,
                triangle_offset: self.geometries[source].first_vertex / 3,
                transform: [
                    mw[0][0], mw[1][0], mw[2][0], mw[3][0], mw[0][1], mw[1][1], mw[2][1], mw[3][1],
                    mw[0][2], mw[1][2], mw[2][2], mw[3][2],
                ],
            });
        }
        self.prepared.sdata.extend_from_slice(&self.world_data);
    }
    pub fn refresh_world(&mut self) {
        self.world_data=self.scene.world.pack(&self.prepared.lights);
        self.prepared.sdata.truncate(self.prepared.lights.len()*4+self.placements.len()*9);
        self.prepared.sdata.extend_from_slice(&self.world_data);
    }
    pub fn object_bounds(&self, i: usize) -> (Vec3, Vec3) {
        let (a, b) = self.local_bounds[self.object_indices[i]];
        (0..8)
            .map(|k| {
                point(
                    self.transforms[i],
                    v3(
                        if k & 1 == 0 { a.x } else { b.x },
                        if k & 2 == 0 { a.y } else { b.y },
                        if k & 4 == 0 { a.z } else { b.z },
                    ),
                )
            })
            .fold(
                (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
                |(lo, hi), p| (lo.min(p), hi.max(p)),
            )
    }
    pub fn bounds(&self) -> (Vec3, Vec3) {
        (0..self.transforms.len())
            .map(|i| self.object_bounds(i))
            .fold(
                (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
                |(lo, hi), (a, b)| (lo.min(a), hi.max(b)),
            )
    }
    pub fn pick(&self, x: f32, y: f32, width: u32, height: u32) -> Option<serde_json::Value> {
        let c = self.scene.camera;
        let eye = GVec3::new(c.position.x, c.position.y, c.position.z);
        let (f,right,up)=c.basis();let f=GVec3::from(f.to_array());let right=GVec3::from(right.to_array());let up=GVec3::from(up.to_array());
        let ty = (c.fov.to_radians() * 0.5).tan();
        let d = (f + right * ((x + 0.5) / width as f32 * 2. - 1.) * ty * width as f32
            / height as f32
            - up * ((y + 0.5) / height as f32 * 2. - 1.) * ty)
            .normalize();
        let mut best = f32::MAX;
        let mut hit = None;
        for (i, &source) in self.object_indices.iter().enumerate() {
            let o=&self.scene.objects[source];
            let inv = self.transforms[i].inverse();
            let ro = inv.transform_point3(eye);
            let rd = inv.transform_vector3(d);
            for t in &o.mesh.indices {
                let a = o.mesh.positions[t[0] as usize];
                let b = o.mesh.positions[t[1] as usize];
                let c = o.mesh.positions[t[2] as usize];
                let a = GVec3::new(a.x, a.y, a.z);
                let e1 = GVec3::new(b.x, b.y, b.z) - a;
                let e2 = GVec3::new(c.x, c.y, c.z) - a;
                let p = rd.cross(e2);
                let det = e1.dot(p);
                if det.abs() < 1e-9 {
                    continue;
                }
                let tv = ro - a;
                let u = tv.dot(p) / det;
                if !(0.0..=1.0).contains(&u) {
                    continue;
                }
                let q = tv.cross(e1);
                let v = rd.dot(q) / det;
                if v < 0. || u + v > 1. {
                    continue;
                }
                let distance = e2.dot(q) / det;
                if distance <= 0. || distance >= best {
                    continue;
                }
                best = distance;
                let p = eye + d * distance;
                let ns = t.map(|j| {
                    let n = o.mesh.normals[j as usize];
                    GVec3::new(n.x, n.y, n.z)
                });
                let n = Mat3::from_mat4(inv.transpose())
                    * (ns[0] * (1. - u - v) + ns[1] * u + ns[2] * v);
                let n = n.normalize();
                hit = Some(
                    serde_json::json!({"layer":self.identities[source].layer,"part":self.identities[source].part,"object":source,"point":p.to_array(),"normal":n.to_array(),"distance":distance}),
                );
            }
        }
        hit
    }
}
trait Metadata {
    fn clone_without_objects(&self) -> Scene;
}
impl Metadata for Scene {
    fn clone_without_objects(&self) -> Scene {
        Scene {
            meters_per_unit:self.meters_per_unit,
            instances:vec![], scatters:vec![], terrains:vec![], displacements:vec![],
            world: self.world.clone(),
            objects: vec![],
            materials: self.materials.clone(),
            lights: self.lights.clone(),
            camera: self.camera,
            backdrop: self.backdrop,
            environment: self.environment,
            exposure: self.exposure,
            shadow_catcher: self.shadow_catcher,
            extra_lights: self.extra_lights.clone(),
            layers: self.layers.clone(),
        }
    }
}
pub fn validate(s: &Scene) -> Result<(), String> {
    s.world.validate()?;
    if !s.meters_per_unit.is_finite()||s.meters_per_unit<=0.{return Err("scene scale must be positive metres per unit".into());}
    for i in &s.instances {i.validate(s.objects.len())?;}
    if s.objects.len() > 65535 {
        return Err("scene needs 1–65535 objects".into());
    }
    if s.materials.is_empty() {
        return Err("scene needs materials".into());
    }
    validate_camera(&s.camera)?;
    for o in &s.objects {
        let m = &o.mesh;
        if o.material >= s.materials.len()
            || m.positions.is_empty()
            || m.indices.is_empty()
            || m.normals.len() != m.positions.len()
            || m.uvs.len() != m.positions.len()
            || !m.colors.is_empty() && m.colors.len()!=m.positions.len()
            || !m.triangle_materials.is_empty() && (m.triangle_materials.len()!=m.indices.len() || m.triangle_materials.iter().any(|&i|i>=s.materials.len()))
        {
            return Err("invalid mesh attributes or material index".into());
        }
        if m.positions
            .iter()
            .chain(m.normals.iter()).chain(m.colors.iter())
            .any(|p| !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite())
            || m.indices
                .iter()
                .flatten()
                .any(|&i| i as usize >= m.positions.len())
        {
            return Err("invalid mesh coordinates or indices".into());
        }
    }
    Ok(())
}

pub fn validate_camera(c: &pm_studio::scene::Camera) -> Result<(), String> {
    if !c.fov.is_finite()
        || !(0.1..179.).contains(&c.fov)
        || (c.target - c.position).length() < 1e-6
    {
        return Err("invalid camera".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transform_patch_reuses_geometry_and_updates_bounds() {
        let s = pm_studio::scene::preset("hero", "blue", 0.).unwrap();
        let mut r = ResidentScene::new(s, vec![], &[], vec![]).unwrap();
        let before = r.prepared.positions.clone();
        let (lo, hi) = r.object_bounds(0);
        let mut m = IDENTITY;
        m[12] = 42.;
        r.patch_transforms(&[Transform {
            object: 0,
            matrix: m,
        }])
        .unwrap();
        assert_eq!(r.prepared.positions, before);
        let (a, b) = r.object_bounds(0);
        assert!((a.x - lo.x - 42.).abs() < 1e-3);
        assert!((b.x - hi.x - 42.).abs() < 1e-3);
        assert!(
            r.patch_transforms(&[Transform {
                object: 0,
                matrix: [0.; 16]
            }])
            .is_err()
        );
    }
    #[test]
    fn scene_round_trip_and_pick() {
        let s = pm_studio::scene::preset("chrome", "blue", 0.).unwrap();
        let s: Scene = serde_json::from_value(serde_json::to_value(s).unwrap()).unwrap();
        let r = ResidentScene::new(s, vec![], &[], vec![]).unwrap();
        assert!(r.pick(128., 72., 256, 144).is_some());
    }
}
