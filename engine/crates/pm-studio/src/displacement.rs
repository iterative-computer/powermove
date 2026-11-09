//! True geometric displacement, evaluated once per prototype before BVH build.
use crate::{math::Vec3, mesh::Mesh};
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Displacement {
    pub object: usize,
    pub width: u32,
    pub height: u32,
    /// Row-major normalized heights; the first row is UV v=0.
    pub values: Vec<f32>,
    /// Scene units, measured along the source vertex normal.
    pub strength: f32,
    #[serde(default = "mid_level")]
    pub mid_level: f32,
    /// Uniform subdivision before displacement; bounded to keep preparation small.
    #[serde(default)]
    pub subdivision: u32,
}
fn mid_level() -> f32 {
    0.5
}
impl Displacement {
    pub fn validate(&self, objects: usize, mesh: &Mesh) -> Result<(), String> {
        if self.object >= objects
            || self.width == 0
            || self.height == 0
            || self.width > 4096
            || self.height > 4096
            || self.values.len() != self.width as usize * self.height as usize
            || self
                .values
                .iter()
                .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
            || !self.strength.is_finite()
            || !self.mid_level.is_finite()
            || !(0. ..=1.).contains(&self.mid_level)
            || self.subdivision > 5
        {
            return Err("invalid mesh displacement".into());
        }
        if mesh
            .indices
            .len()
            .checked_mul(4usize.pow(self.subdivision))
            .is_none_or(|n| n > 1_000_000)
        {
            return Err("displacement exceeds one million triangles per prototype".into());
        }
        Ok(())
    }
    fn height_at(&self, uv: [f32; 2]) -> f32 {
        let x = uv[0].clamp(0., 1.) * (self.width - 1) as f32;
        let y = uv[1].clamp(0., 1.) * (self.height - 1) as f32;
        let ix = x.floor() as u32;
        let iy = y.floor() as u32;
        let u = x - ix as f32;
        let v = y - iy as f32;
        let at = |a: u32, b: u32| {
            self.values[(b.min(self.height - 1) * self.width + a.min(self.width - 1)) as usize]
        };
        (at(ix, iy) * (1. - u) + at(ix + 1, iy) * u) * (1. - v)
            + (at(ix, iy + 1) * (1. - u) + at(ix + 1, iy + 1) * u) * v
    }
    pub fn apply(&self, mesh: &Mesh) -> Mesh {
        let mut m = mesh.clone();
        for _ in 0..self.subdivision {
            let old = std::mem::take(&mut m.indices);
            let old_materials = std::mem::take(&mut m.triangle_materials);
            let mut edges = std::collections::HashMap::new();
            for (face, [a, b, c]) in old.into_iter().enumerate() {
                let mut midpoint = |a: u32, b: u32| -> u32 {
                    let key = if a < b { (a, b) } else { (b, a) };
                    *edges.entry(key).or_insert_with(|| {
                        let (a, b) = (a as usize, b as usize);
                        let i = m.positions.len() as u32;
                        m.positions.push((m.positions[a] + m.positions[b]) * 0.5);
                        m.normals.push((m.normals[a] + m.normals[b]).normalize());
                        m.uvs.push([
                            (m.uvs[a][0] + m.uvs[b][0]) * 0.5,
                            (m.uvs[a][1] + m.uvs[b][1]) * 0.5,
                        ]);
                        if !m.colors.is_empty() { m.colors.push((m.colors[a]+m.colors[b])*0.5); }
                        i
                    })
                };
                let ab = midpoint(a, b);
                let bc = midpoint(b, c);
                let ca = midpoint(c, a);
                if let Some(&material)=old_materials.get(face) { m.triangle_materials.extend([material;4]); }
                m.indices
                    .extend([[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]);
            }
        }
        for i in 0..m.positions.len() {
            m.positions[i] = m.positions[i]
                + m.normals[i].normalize()
                    * ((self.height_at(m.uvs[i]) - self.mid_level) * self.strength);
        }
        let reference = m.normals.clone();
        m.normals.fill(Vec3::ZERO);
        for t in &m.indices {
            let [a, b, c] = t.map(|i| m.positions[i as usize]);
            let mut n = (b - a).cross(c - a);
            let expected = t
                .iter()
                .fold(Vec3::ZERO, |sum, &i| sum + reference[i as usize]);
            if n.dot(expected) < 0. {
                n = -n;
            }
            for &i in t {
                m.normals[i as usize] = m.normals[i as usize] + n;
            }
        }
        for (i, n) in m.normals.iter_mut().enumerate() {
            *n = if n.length() > 1e-10 {
                n.normalize()
            } else {
                reference[i]
            };
        }
        m
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::v3;
    #[test]
    fn height_moves_geometry_and_subdivision_reuses_edges() {
        let m = Mesh {
            positions: vec![v3(0., 0., 0.), v3(1., 0., 0.), v3(0., 0., 1.)],
            normals: vec![Vec3::Y; 3],
            uvs: vec![[0., 0.], [1., 0.], [0., 1.]],
            indices: vec![[0, 2, 1]],
            ..Default::default()
        };
        let d = Displacement {
            object: 0,
            width: 2,
            height: 2,
            values: vec![1.; 4],
            strength: 2.,
            mid_level: 0.5,
            subdivision: 2,
        };
        d.validate(1, &m).unwrap();
        let a = d.apply(&m);
        assert_eq!(a.indices.len(), 16);
        assert_eq!(a.positions.len(), 15);
        assert!(a.positions.iter().all(|p| (p.y - 1.).abs() < 1e-6));
        assert!(a.normals.iter().all(|n| (n.y - 1.).abs() < 1e-6));
    }
    #[test]
    fn rejects_explosive_subdivision() {
        let mut m = Mesh::default();
        m.indices.resize(2000, [0, 0, 0]);
        let d = Displacement {
            object: 0,
            width: 1,
            height: 1,
            values: vec![1.],
            strength: 1.,
            mid_level: 0.,
            subdivision: 5,
        };
        assert!(d.validate(1, &m).is_err());
    }
}
