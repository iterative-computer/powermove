//! Fast path for a scene already prepared by Studio's shared TS extractor.
//! This is the binary layout of Rust's Prepared scene, not a second renderer model.
use crate::{protocol::Range, scene::Identity};
use glam::{Mat4, Vec3};
use pm_studio::{
    bvh::WideNode,
    gpu::{RtGeometry, RtPlacement},
    math::{Vec3 as V, v3},
    scene::{Camera, GpuMaterial, GpuTri, Prepared, Scene},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraBasis {
    pub position: [f32; 3],
    pub forward: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub tan_y: f32,
    pub aspect: f32,
    pub lens_radius: f32,
    pub focus: f32,
    #[serde(default)]
    pub orthographic: bool,
    pub near: Option<f32>,
    pub far: Option<f32>,
    #[serde(default="default_iris")]
    pub iris:[f32;4],
}
fn default_iris()->[f32;4]{[0.,0.,0.,1.]}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Environment {
    pub zenith: [f32; 3],
    pub horizon: [f32; 3],
    pub ground: [f32; 3],
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub camera: CameraBasis,
    pub exposure: f32,
    pub backdrop: [f32; 3],
    pub backdrop_opaque: bool,
    pub shadow_catcher: bool,
    pub shadows: bool,
    pub floor_y: f32,
    pub environment: Environment,
    pub bounds: [f32; 4],
    #[serde(default)]
    pub ambient:[f32;3],
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceIdentity {
    pub part: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Upload {
    pub key: String,
    pub positions: Range,
    pub tris: Range,
    pub nodes: Range,
    pub materials: Range,
    pub sdata: Range,
    pub top_nodes: usize,
    pub instance_count: usize,
    pub light_count: usize,
    pub objects: Vec<Identity>,
    #[serde(default)]
    pub instance_policies: Vec<InstanceIdentity>,
    pub view: View,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Patch {
    pub key: String,
    pub top: Range,
    pub sdata: Range,
    pub materials: Range,
    pub view: View,
    pub instance_policies: Option<Vec<InstanceIdentity>>,
}

pub struct PackedScene {
    pub key: String,
    pub prepared: Prepared,
    pub geometries: Vec<RtGeometry>,
    pub placements: Vec<RtPlacement>,
    pub top_nodes: usize,
    pub view: View,
    pub objects: Vec<Identity>,
    pub parts: Vec<Option<String>>,
    local_bounds: Vec<(Vec3, Vec3)>,
}
fn v(a: [f32; 3]) -> V {
    v3(a[0], a[1], a[2])
}
impl View {
    pub fn scene(&self) -> Scene {
        let c = &self.camera;
        let pos = v(c.position);
        Scene {
            meters_per_unit:0.1,
            instances:vec![], scatters:vec![], terrains:vec![], displacements:vec![],
            world: Default::default(),
            objects: vec![],
            materials: vec![],
            lights: vec![],
            camera: Camera {
                position: pos,
                target: pos + v(c.forward) * c.focus.max(1.),
                fov: 2. * c.tan_y.atan().to_degrees(),
                f_stop: 0.,
                focus: None,
                focal_mm: 50.,
            },
            backdrop: v(self.backdrop),
            environment: (v(self.environment.zenith), v(self.environment.horizon)),
            exposure: self.exposure,
            shadow_catcher: self.shadow_catcher,
            extra_lights: vec![],
            layers: None,
        }
    }
    pub fn basis(&self) -> [[f32; 4]; 4] {
        let c = &self.camera;
        [
            v(c.position).to4(c.lens_radius),
            v(c.forward).to4(c.focus),
            v(c.right).to4(c.tan_y * c.aspect),
            v(c.up).to4(c.tan_y),
        ]
    }
    pub fn ground(&self) -> [f32; 4] {
        v(self.environment.ground).to4(self.floor_y)
    }
    fn validate(&self) -> Result<(), String> {
        let c = &self.camera;
        if !self.exposure.is_finite()
            || self.exposure < 0.
            || !c.tan_y.is_finite()
            || c.tan_y <= 0.
            || !c.aspect.is_finite()
            || c.aspect <= 0.
            || self.ambient.iter().any(|x|!x.is_finite()||*x<0.)
            || c.iris.iter().any(|x|!x.is_finite())
            || c.iris[0]<0. || c.iris[0]>16. || c.iris[2]<0. || c.iris[2]>1. || c.iris[3]<0.1 || c.iris[3]>10.
            || c.position
                .iter()
                .chain(c.forward.iter())
                .chain(c.up.iter())
                .chain(c.right.iter())
                .chain(self.bounds.iter())
                .any(|x| !x.is_finite())
        {
            return Err("invalid native camera/view".into());
        }
        Ok(())
    }
}
impl PackedScene {
    pub fn new(u: Upload, binary: &[u8]) -> Result<Self, String> {
        u.view.validate()?;
        let positions = u.positions.typed::<[f32; 4]>(binary)?;
        let tris = u.tris.typed::<GpuTri>(binary)?;
        let nodes = u.nodes.typed::<WideNode>(binary)?;
        let materials = u.materials.typed::<GpuMaterial>(binary)?;
        let sdata = u.sdata.typed::<[f32; 4]>(binary)?;
        if u.key.len() > 65536
            || positions.len() != tris.len() * 3
            || materials.is_empty()
            || u.top_nodes == 0
            || u.top_nodes > nodes.len()
            || u.instance_count > 1_000_000
            || u.light_count > 65535
            || (sdata.len() < u.light_count * 4 + u.instance_count * 9
                && !(u.light_count == 0 && u.instance_count == 0 && sdata.len() == 1))
        {
            return Err("invalid prepared scene buffer sizes".into());
        }
        let trailer_start=u.light_count*4+u.instance_count*9;
        if sdata.len()>trailer_start && !(u.light_count==0 && u.instance_count==0 && sdata.len()==1) {pm_studio::environment::validate_packed(&sdata[trailer_start..],u.light_count)?;}
        if tris.iter().any(|t| !triangle_shading_is_finite(t)) || positions
            .iter()
            .flatten()
            .chain(bytemuck::cast_slice::<GpuMaterial, f32>(&materials))
            .chain(sdata.iter().flatten())
            .any(|x| !x.is_finite())
        {
            return Err("prepared scene contains non-finite data".into());
        }
        for t in tris.iter().filter(|_| u.instance_count > 0) {
            if t.uv2m[2] < 0.
                || t.uv2m[2] as usize >= materials.len()
                || t.uv2m[3] < 0.
                || t.uv2m[3] as usize >= u.objects.len()
            {
                return Err("prepared triangle has invalid material/object index".into());
            }
        }
        validate_instance_flags(&sdata,u.light_count,u.instance_count,u.objects.len())?;
        validate_top(&nodes[..u.top_nodes], u.instance_count)?;
        let lights = sdata[..u.light_count * 4]
            .chunks_exact(4)
            .map(|s| pm_studio::scene::GpuLight {
                pos: s[0],
                u: s[1],
                v: s[2],
                radiance: s[3],
            })
            .collect();
        let prepared = Prepared {
            instancing:None,
            positions,
            tris,
            nodes,
            materials,
            lights,
            sdata,
        };
        let (geometries, placements) = placements(&prepared, u.instance_count)?;
        let parts = if u.instance_policies.is_empty() {
            vec![None; u.instance_count]
        } else {
            if u.instance_policies.len() != u.instance_count {
                return Err("instance identities do not match instances".into());
            }
            u.instance_policies.into_iter().map(|p| p.part).collect()
        };
        let local_bounds = geometries
            .iter()
            .map(|g| {
                prepared.positions
                    [g.first_vertex as usize..(g.first_vertex + g.vertex_count) as usize]
                    .iter()
                    .map(|p| Vec3::from_slice(p))
                    .fold(
                        (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
                        |(a, b), p| (a.min(p), b.max(p)),
                    )
            })
            .collect();
        Ok(Self {
            key: u.key,
            prepared,
            geometries,
            placements,
            top_nodes: u.top_nodes,
            view: u.view,
            objects: u.objects,
            parts,
            local_bounds,
        })
    }
    pub fn patch_view(&mut self,key:&str,view:View)->Result<bool,String>{
        if key!=self.key{return Err("native geometry key is stale; upload scene again".into());}
        view.validate()?;
        let mut previous=self.view.clone();previous.camera=view.camera.clone();
        let content_changed=self.view.camera.orthographic!=view.camera.orthographic || previous!=view;
        self.view=view;Ok(content_changed)
    }
    pub fn patch(&mut self, p: Patch, binary: &[u8]) -> Result<bool, String> {
        if self.key != p.key {
            return Err("native geometry key is stale; upload scene again".into());
        }
        p.view.validate()?;
        if p.instance_policies
            .as_ref()
            .is_some_and(|policies| policies.len() != self.placements.len())
        {
            return Err("instance identities do not match instances".into());
        }
        let nodes = p.top.typed::<WideNode>(binary)?;
        let sdata = p.sdata.typed::<[f32; 4]>(binary)?;
        let materials = p.materials.typed::<GpuMaterial>(binary)?;
        if nodes.len() != self.top_nodes
            || sdata.len() != self.prepared.sdata.len()
            || materials.len() != self.prepared.materials.len()
        {
            return Err("prepared patch changes buffer sizes; upload scene again".into());
        }
        if sdata
            .iter()
            .flatten()
            .chain(bytemuck::cast_slice::<GpuMaterial, f32>(&materials))
            .any(|x| !x.is_finite())
        {
            return Err("patch contains non-finite data".into());
        }
        let lights=self.prepared.lights.len();let count=self.placements.len();let trailer=lights*4+count*9;
        if sdata.len()>trailer && !(lights==0&&count==0&&sdata.len()==1){pm_studio::environment::validate_packed(&sdata[trailer..],lights)?;}
        validate_instance_flags(&sdata,lights,count,self.objects.len())?;
        validate_top(&nodes, self.placements.len())?;
        let mut old_view = self.view.clone();
        old_view.camera = p.view.camera.clone();
        let content_changed = self.view.camera.orthographic != p.view.camera.orthographic
            || old_view != p.view
            || sdata != self.prepared.sdata
            || bytemuck::cast_slice::<GpuMaterial, u8>(&materials)
                != bytemuck::cast_slice::<GpuMaterial, u8>(&self.prepared.materials);
        // Camera-only patches retain the complete BLAS/placement cache.
        if sdata == self.prepared.sdata {
            self.prepared.nodes[..self.top_nodes].copy_from_slice(&nodes);
            self.prepared.materials = materials;
            self.view = p.view;
            if let Some(policies) = p.instance_policies {
                self.parts = policies.into_iter().map(|p| p.part).collect();
            }
            return Ok(content_changed);
        }
        let old = std::mem::replace(&mut self.prepared.sdata, sdata);
        let count = self.placements.len();
        let (geometries, placements) = match placements(&self.prepared, count) {
            Ok(p) => p,
            Err(e) => {
                self.prepared.sdata = old;
                return Err(e);
            }
        };
        if geometries.len() != self.geometries.len()
            || geometries
                .iter()
                .zip(&self.geometries)
                .any(|(a, b)| a.first_vertex != b.first_vertex || a.vertex_count != b.vertex_count)
        {
            self.prepared.sdata = old;
            return Err("prepared patch changes resident geometry; upload scene again".into());
        }
        // Geometry order is sorted by BLAS root, so transforms never reorder Metal BLAS.
        self.placements = placements;
        self.prepared.nodes[..self.top_nodes].copy_from_slice(&nodes);
        self.prepared.materials = materials;
        self.view = p.view;
        if let Some(policies) = p.instance_policies {
            self.parts = policies.into_iter().map(|p| p.part).collect();
        }
        for (i, l) in self.prepared.lights.iter_mut().enumerate() {
            let s = &self.prepared.sdata[i * 4..i * 4 + 4];
            l.pos = s[0];
            l.u = s[1];
            l.v = s[2];
            l.radiance = s[3];
        }
        Ok(content_changed)
    }
    pub fn bounds(&self, layer: Option<&str>) -> Option<(Vec3, Vec3)> {
        let mut bounds: Option<(Vec3, Vec3)> = None;
        for p in &self.placements {
            let oi = self.prepared.tris[p.triangle_offset as usize].uv2m[3] as usize;
            if layer.is_some_and(|l| self.objects[oi].layer != l) {
                continue;
            }
            let (a, b) = self.local_bounds[p.geometry];
            let t = p.transform;
            let world = Mat4::from_cols_array(&[
                t[0], t[4], t[8], 0., t[1], t[5], t[9], 0., t[2], t[6], t[10], 0., t[3], t[7],
                t[11], 1.,
            ]);
            for k in 0..8 {
                let p = world.transform_point3(Vec3::new(
                    if k & 1 == 0 { a.x } else { b.x },
                    if k & 2 == 0 { a.y } else { b.y },
                    if k & 4 == 0 { a.z } else { b.z },
                ));
                bounds = Some(match bounds {
                    None => (p, p),
                    Some((a, b)) => (a.min(p), b.max(p)),
                });
            }
        }
        bounds
    }
    pub fn pick(&self, x: f32, y: f32, w: u32, h: u32) -> Option<serde_json::Value> {
        let c = &self.view.camera;
        let mut eye = Vec3::from(c.position);
        let direction = (Vec3::from(c.forward)
            + Vec3::from(c.right) * ((x + 0.5) / w as f32 * 2. - 1.) * c.tan_y * c.aspect
            - Vec3::from(c.up) * ((y + 0.5) / h as f32 * 2. - 1.) * c.tan_y)
            .normalize();
        let direction = if c.orthographic {
            eye += Vec3::from(c.right) * ((x + 0.5) / w as f32 * 2. - 1.) * c.tan_y * c.aspect
                - Vec3::from(c.up) * ((y + 0.5) / h as f32 * 2. - 1.) * c.tan_y;
            Vec3::from(c.forward)
        } else {
            direction
        };
        let mut best = f32::MAX;
        let mut found = None;
        for (instance, placement) in self.placements.iter().enumerate() {
            let a = placement.transform;
            let world = Mat4::from_cols_array(&[
                a[0], a[4], a[8], 0., a[1], a[5], a[9], 0., a[2], a[6], a[10], 0., a[3], a[7],
                a[11], 1.,
            ]);
            let inv = world.inverse();
            let ro = inv.transform_point3(eye);
            let rd = inv.transform_vector3(direction);
            let geometry = &self.geometries[placement.geometry];
            for t in placement.triangle_offset as usize
                ..(placement.triangle_offset + geometry.vertex_count / 3) as usize
            {
                let pos = &self.prepared.positions[t * 3..t * 3 + 3];
                let a = Vec3::from_slice(&pos[0]);
                let e1 = Vec3::from_slice(&pos[1]) - a;
                let e2 = Vec3::from_slice(&pos[2]) - a;
                let p = rd.cross(e2);
                let det = e1.dot(p);
                if det.abs() < 1e-10 {
                    continue;
                }
                let tv = ro - a;
                let u = tv.dot(p) / det;
                if !(0.0..=1.0).contains(&u) {
                    continue;
                }
                let q = tv.cross(e1);
                let vv = rd.dot(q) / det;
                if vv < 0. || u + vv > 1. {
                    continue;
                }
                let d = e2.dot(q) / det;
                if d <= 0. || d >= best {
                    continue;
                }
                best = d;
                let tri = &self.prepared.tris[t];
                let n = Vec3::from_slice(&tri.n[0]) * (1. - u - vv)
                    + Vec3::from_slice(&tri.n[1]) * u
                    + Vec3::from_slice(&tri.n[2]) * vv;
                let n = inv.transpose().transform_vector3(n).normalize();
                let oi = tri.uv2m[3] as usize;
                let identity = &self.objects[oi];
                found = Some(
                    serde_json::json!({"layer":identity.layer,"part":self.parts[instance].as_deref().unwrap_or(&identity.part),"object":oi,"point":(eye+direction*d).to_array(),"normal":n.to_array(),"distance":d}),
                );
            }
        }
        found
    }
}
fn validate_instance_flags(data:&[[f32;4]],lights:usize,count:usize,objects:usize)->Result<(),String>{
    for i in 0..count {
        let flags=data[lights*4+i*9+4][3];let hidden=data[lights*4+i*9+5][3];
        if !flags.is_finite()||flags<0.||flags.fract()!=0.||(flags as u32>>1)>objects as u32||!(0. ..=1.).contains(&hidden) {return Err("invalid instance flags or output-layer override".into());}
    }Ok(())
}
fn placements(p: &Prepared, count: usize) -> Result<(Vec<RtGeometry>, Vec<RtPlacement>), String> {
    let start = p.lights.len() * 4;
    let mut roots = BTreeMap::<u32, (u32, u32)>::new();
    for i in 0..count {
        let root = p.sdata[start + i * 9 + 3][3];
        if root < 0. || root.fract() != 0. {
            return Err("invalid instance root".into());
        }
        let root = root as u32;
        if let std::collections::btree_map::Entry::Vacant(e) = roots.entry(root) {
            e.insert(triangle_range(&p.nodes, root, p.tris.len())?);
        }
    }
    let geometries = roots
        .values()
        .map(|&(lo, hi)| RtGeometry {
            first_vertex: lo * 3,
            vertex_count: (hi - lo) * 3,
        })
        .collect::<Vec<_>>();
    let index = roots
        .keys()
        .enumerate()
        .map(|(i, &r)| (r, i))
        .collect::<BTreeMap<_, _>>();
    let mut out = Vec::new();
    for i in 0..count {
        let rows = &p.sdata[start + i * 9..start + i * 9 + 9];
        let root = rows[3][3] as u32;
        let inv = Mat4::from_cols_array(&[
            rows[0][0], rows[1][0], rows[2][0], 0., rows[0][1], rows[1][1], rows[2][1], 0.,
            rows[0][2], rows[1][2], rows[2][2], 0., rows[0][3], rows[1][3], rows[2][3], 1.,
        ]);
        if !inv.is_finite() || inv.determinant().abs() < 1e-12 {
            return Err("invalid instance transform".into());
        }
        let m = inv.inverse().to_cols_array_2d();
        out.push(RtPlacement {
            geometry: index[&root],
            triangle_offset: roots[&root].0,
            transform: [
                m[0][0], m[1][0], m[2][0], m[3][0], m[0][1], m[1][1], m[2][1], m[3][1], m[0][2],
                m[1][2], m[2][2], m[3][2],
            ],
        });
    }
    Ok((geometries, out))
}
fn triangle_shading_is_finite(triangle: &GpuTri) -> bool {
    // Normal.w stores packed RGBA8 bits (including 0xffffffff for white), not
    // a floating-point value. Validate only the actual shading coordinates.
    triangle.n.iter().flat_map(|n| n[..3].iter())
        .chain(triangle.uv01.iter()).chain(triangle.uv2m.iter())
        .chain(triangle.tangent.iter()).chain(triangle.uv01_secondary.iter()).chain(triangle.uv2_secondary.iter()).chain(triangle.tangent_secondary.iter()).all(|v| v.is_finite())
}
fn triangle_range(nodes: &[WideNode], root: u32, triangles: usize) -> Result<(u32, u32), String> {
    let mut stack = vec![(root, 0usize)];
    let mut visited = std::collections::HashSet::new();
    let (mut lo, mut hi) = (u32::MAX, 0);
    while let Some((i, depth)) = stack.pop() {
        if depth >= 60 || !visited.insert(i) {
            return Err("cyclic/duplicate or excessively deep BLAS node".into());
        }
        let n = nodes
            .get(i as usize)
            .ok_or("BLAS root outside node buffer")?;
        if n.lmin.iter().chain(n.lmax.iter()).chain(n.rmin.iter()).chain(n.rmax.iter()).any(|v| !v.is_finite()) {
            return Err("BLAS contains non-finite bounds".into());
        }
        for (child, count) in [(n.left, n.lcount), (n.right, n.rcount)] {
            if count == u32::MAX {
                continue;
            }
            if count == 0 {
                stack.push((child, depth + 1));
            } else {
                let end = child.checked_add(count).ok_or("triangle range overflow")?;
                if end as usize > triangles {
                    return Err("BLAS triangle range outside buffer".into());
                }
                lo = lo.min(child);
                hi = hi.max(end);
            }
        }
    }
    if lo >= hi {
        return Err("empty BLAS".into());
    }
    Ok((lo, hi))
}

// Reject malformed trees before they can hang a GPU traversal or overflow its
// fixed stack. Padding nodes are intentionally not visited.
fn validate_top(nodes: &[WideNode], instances: usize) -> Result<(), String> {
    if instances == 0 {
        return Ok(());
    }
    let mut stack = vec![(0usize, 0usize)];
    let mut visited = std::collections::HashSet::new();
    let mut seen = vec![false; instances];
    while let Some((i, depth)) = stack.pop() {
        if depth >= 60 || !visited.insert(i) {
            return Err("cyclic or excessively deep TLAS".into());
        }
        let n = nodes.get(i).ok_or("TLAS child outside top-level buffer")?;
        if n.lmin
            .iter()
            .chain(n.lmax.iter())
            .chain(n.rmin.iter())
            .chain(n.rmax.iter())
            .any(|v| !v.is_finite())
        {
            return Err("TLAS contains non-finite bounds".into());
        }
        for (child, count) in [(n.left, n.lcount), (n.right, n.rcount)] {
            if count == u32::MAX {
                continue;
            }
            if count == 0 {
                stack.push((child as usize, depth + 1));
                continue;
            }
            let end = child.checked_add(count).ok_or("TLAS range overflow")?;
            let range = seen
                .get_mut(child as usize..end as usize)
                .ok_or("TLAS instance range outside scene")?;
            for flag in range {
                if *flag {
                    return Err("duplicate TLAS instance".into());
                }
                *flag = true;
            }
        }
    }
    if seen.iter().any(|v| !*v) {
        return Err("TLAS omits scene instances".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packed_vertex_colour_bits_are_not_floating_point_shading() {
        let good = GpuTri { n: [[0., 1., 0., f32::from_bits(0xffffffff)]; 3], uv01: [0.; 4], uv2m: [0.; 4], tangent: [1., 0., 0., 1.], uv01_secondary:[0.;4], uv2_secondary:[0.;4], tangent_secondary:[1.,0.,0.,1.] };
        assert!(triangle_shading_is_finite(&good));
        let mut bad = good; bad.n[0][0] = f32::NAN;
        assert!(!triangle_shading_is_finite(&bad));
        let mut bad = good; bad.uv01[0] = f32::INFINITY;
        assert!(!triangle_shading_is_finite(&bad));
        let mut bad=good;bad.uv01_secondary[0]=f32::NAN;
        assert!(!triangle_shading_is_finite(&bad));
    }
    #[test]
    fn rejects_deep_blas_and_non_finite_bounds() {
        let leaf = WideNode { left: 0, lcount: 1, rcount: u32::MAX, ..Default::default() };
        assert_eq!(triangle_range(&[leaf], 0, 1).unwrap(), (0, 1));
        let mut bad = leaf;
        bad.lmin[0] = f32::NAN;
        assert!(triangle_range(&[bad], 0, 1).is_err());
        let mut deep = vec![leaf; 65];
        for (i, n) in deep.iter_mut().enumerate().take(64) {
            n.left = (i + 1) as u32; n.lcount = 0; n.right = 0; n.rcount = 1;
        }
        assert!(triangle_range(&deep, 0, 1).is_err());
    }
    #[test]
    fn layer_flags_preserve_visibility_and_reject_out_of_range_ids(){
        let mut data=vec![[0.;4];9];data[4][3]=2.;data[5][3]=1.;validate_instance_flags(&data,0,1,1).unwrap();
        data[4][3]=4.;assert!(validate_instance_flags(&data,0,1,1).is_err());data[4][3]=1.25;assert!(validate_instance_flags(&data,0,1,1).is_err());
    }
    #[test]
    fn rejects_gpu_traversal_cycles_and_invalid_leaves() {
        assert!(validate_top(&[WideNode::default()], 1).is_err());
        let mut n = WideNode {
            lcount: 1,
            rcount: u32::MAX,
            ..Default::default()
        };
        assert!(validate_top(&[n], 1).is_ok());
        n.left = 1;
        assert!(validate_top(&[n], 1).is_err());
        n.left = 0;
        n.rcount = 1;
        assert!(validate_top(&[n], 1).is_err());
    }
}
