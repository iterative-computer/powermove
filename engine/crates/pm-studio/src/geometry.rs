//! One BLAS per prototype and one TLAS over all placements. The portable and
//! hardware paths read the same transforms and triangle offsets.
use crate::{
    bvh,
    gpu::{RtGeometry, RtPlacement},
    math::{Vec3, v3},
    scatter::Instance,
    scene::{Prepared, Scene},
};
use glam::{Mat4, Vec3 as G};
#[derive(Clone, Debug)]
pub struct Instancing {
    pub geometries: Vec<RtGeometry>,
    pub placements: Vec<RtPlacement>,
    pub top_nodes: usize,
}
pub fn matrix(i: &Instance) -> Mat4 {
    let m = i.matrix();
    Mat4::from_cols_array(&[
        m[0], m[4], m[8], 0., m[1], m[5], m[9], 0., m[2], m[6], m[10], 0., m[3], m[7], m[11], 1.,
    ])
}
pub fn transformed_bounds(bounds: (Vec3, Vec3), m: Mat4) -> (Vec3, Vec3) {
    let (a, b) = bounds;
    (0..8)
        .map(|i| {
            let p = m.transform_point3(G::new(
                if i & 1 == 0 { a.x } else { b.x },
                if i & 2 == 0 { a.y } else { b.y },
                if i & 4 == 0 { a.z } else { b.z },
            ));
            v3(p.x, p.y, p.z)
        })
        .fold(
            (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
            |(a, b), p| (a.min(p), b.max(p)),
        )
}
impl Scene {
    /// Resolve terrain meshes and scatter once, before keeping a resident scene.
    /// Explicit instances replace the implicit identity placement of each object.
    pub fn expanded(&self) -> Result<Self, String> {
        let mut s = self.clone();
        for t in &self.terrains {
            t.validate(self.materials.len())?;
        }
        for scatter in &self.scatters {
            scatter.validate(self)?;
        }
        if !self.terrains.is_empty() || !self.scatters.is_empty() {
            if s.instances.is_empty() {
                s.instances = (0..s.objects.len())
                    .map(|object| Instance {
                        object,
                        ..Default::default()
                    })
                    .collect();
            }
            for scatter in &self.scatters {
                s.instances.extend(scatter.generate(self));
            }
            let mut layers = s
                .layers
                .clone()
                .unwrap_or_else(|| (0..s.objects.len() as u32).collect());
            layers.resize(s.objects.len(), 0);
            for terrain in &self.terrains {
                for (material, mesh) in terrain.meshes() {
                    let object = s.objects.len();
                    s.objects.push(crate::scene::Object { mesh, material });
                    layers.push(object as u32);
                    s.instances.push(Instance {
                        object,
                        ..Default::default()
                    });
                }
            }
            s.layers = Some(layers);
            s.terrains.clear();
            s.scatters.clear();
        }
        if s.instances.len() > 1_000_000 {
            return Err("scene exceeds one million instances".into());
        }
        for i in &s.instances {
            i.validate(s.objects.len())?;
        }
        Ok(s)
    }
}
pub fn prepare(s: &Scene) -> Prepared {
    let transforms: Vec<_> = s.instances.iter().map(matrix).collect();
    let local_bounds: Vec<_> = s.objects.iter().map(|o| o.mesh.bounds()).collect();
    let boxes: Vec<_> = s
        .instances
        .iter()
        .zip(&transforms)
        .map(|(i, m)| {
            let (a, b) = transformed_bounds(local_bounds[i.object], *m);
            [a, b, a]
        })
        .collect();
    let (top, order) = bvh::build(&boxes);
    let top = bvh::widen(&top);
    let top_nodes = top.len();
    let mut metadata = s.clone();
    metadata.objects.clear();
    metadata.instances.clear();
    metadata.terrains.clear();
    metadata.scatters.clear();
    // The actual world is packed exactly once, after all placements.
    let world = std::mem::take(&mut metadata.world);
    let mut out = metadata.prepare();
    out.positions.clear();
    out.tris.clear();
    out.nodes = top;
    let mut roots = Vec::new();
    let mut geometries = Vec::new();
    for (index, o) in s.objects.iter().enumerate() {
        let mut one = metadata.clone();
        one.objects = vec![o.clone()];
        one.layers = Some(vec![
            s.layers
                .as_ref()
                .and_then(|l| l.get(index).copied())
                .unwrap_or(index as u32),
        ]);
        let p = one.prepare();
        let tri_offset = out.tris.len() as u32;
        let root = out.nodes.len() as u32;
        roots.push(root);
        geometries.push(RtGeometry {
            first_vertex: tri_offset * 3,
            vertex_count: p.positions.len() as u32,
        });
        out.positions.extend(p.positions);
        out.tris.extend(p.tris);
        for mut n in p.nodes.into_iter().skip(1) {
            for (child, count) in [(&mut n.left, n.lcount), (&mut n.right, n.rcount)] {
                if count != u32::MAX {
                    *child = if count == 0 {
                        *child - 1 + root
                    } else {
                        *child + tri_offset
                    };
                }
            }
            out.nodes.push(n);
        }
    }
    // Restore the sun omitted from the temporary metadata's world.
    if let Some(sun) = world.sun() {
        out.lights.push(sun);
    }
    out.sdata = out
        .lights
        .iter()
        .flat_map(|l| [l.pos, l.u, l.v, l.radiance])
        .collect();
    let mut placements = Vec::with_capacity(order.len());
    for i in order.into_iter().map(|i| i as usize) {
        let inst = &s.instances[i];
        let m = transforms[i];
        let inv = m.inverse();
        let normal = inv.transpose().to_cols_array_2d();
        let iv = inv.to_cols_array_2d();
        let mw = m.to_cols_array_2d();
        for r in 0..3 {
            out.sdata.push([iv[0][r], iv[1][r], iv[2][r], iv[3][r]]);
        }
        for r in 0..3 {
            out.sdata.push([
                normal[0][r],
                normal[1][r],
                normal[2][r],
                match r {
                    0 => roots[inst.object] as f32,
                    2 => inst.layer.map_or(0., |l| (l + 1) as f32),
                    _ => 0.,
                },
            ]);
        }
        for r in 0..3 {
            out.sdata.push([mw[0][r], mw[1][r], mw[2][r], 0.]);
        }
        placements.push(RtPlacement {
            geometry: inst.object,
            triangle_offset: geometries[inst.object].first_vertex / 3,
            transform: inst.matrix(),
        });
    }
    out.sdata.extend(world.pack(&out.lights));
    out.instancing = Some(Instancing {
        geometries,
        placements,
        top_nodes,
    });
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hundred_thousand_instances_keep_one_mesh() {
        let mut s = crate::scene::preset("hero", "blue", 0.).unwrap();
        s.objects.truncate(1);
        s.instances = (0..100000)
            .map(|i| Instance {
                position: v3((i % 400) as f32 * 200., 0., (i / 400) as f32 * 200.),
                ..Default::default()
            })
            .collect();
        let p = s.prepare();
        let meta = p.instancing.as_ref().unwrap();
        assert_eq!(meta.geometries.len(), 1);
        assert_eq!(meta.placements.len(), 100000);
        assert_eq!(p.tris.len(), s.objects[0].mesh.indices.len());
        assert!(p.sdata.len() < 1_000_100);
        assert!(p.nodes.len() < 110000);
        assert!(meta.placements.iter().all(|i| i.geometry == 0));
    }
}
