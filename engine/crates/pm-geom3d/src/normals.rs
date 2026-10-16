//! `BufferGeometry.computeVertexNormals` and `normalizeNormals` (three.js
//! r180), on f32 attributes exactly as three.js stores them: every
//! `setXYZ` rounds to f32, arithmetic happens in f64.

/// `Vector3.crossVectors(a, b)`.
pub(crate) fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn at(v: &[f32], i: usize) -> [f64; 3] {
    [
        f64::from(v[i * 3]),
        f64::from(v[i * 3 + 1]),
        f64::from(v[i * 3 + 2]),
    ]
}

fn set(v: &mut [f32], i: usize, x: [f64; 3]) {
    v[i * 3] = x[0] as f32;
    v[i * 3 + 1] = x[1] as f32;
    v[i * 3 + 2] = x[2] as f32;
}

/// The face normal three.js accumulates: `(c − b) × (a − b)`.
fn face(positions: &[f32], a: usize, b: usize, c: usize) -> [f64; 3] {
    let (pa, pb, pc) = (at(positions, a), at(positions, b), at(positions, c));
    let cb = [pc[0] - pb[0], pc[1] - pb[1], pc[2] - pb[2]];
    let ab = [pa[0] - pb[0], pa[1] - pb[1], pa[2] - pb[2]];
    cross(cb, ab)
}

/// `computeVertexNormals()`: area-weighted vertex normals for an indexed
/// geometry, face normals otherwise, then normalized.
pub fn compute_vertex_normals(positions: &[f32], index: Option<&[u32]>) -> Vec<f32> {
    let count = positions.len() / 3;
    let mut normals = vec![0f32; count * 3];
    match index {
        Some(index) => {
            for t in index.chunks_exact(3) {
                let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
                let cb = face(positions, a, b, c);
                for v in [a, b, c] {
                    let n = at(&normals, v);
                    set(&mut normals, v, [n[0] + cb[0], n[1] + cb[1], n[2] + cb[2]]);
                }
            }
        }
        None => {
            let mut i = 0;
            while i + 2 < count {
                let cb = face(positions, i, i + 1, i + 2);
                for v in i..i + 3 {
                    set(&mut normals, v, cb);
                }
                i += 3;
            }
        }
    }
    normalize_normals(&mut normals);
    normals
}

/// `normalizeNormals()`: each normal read from f32, `Vector3.normalize()`
/// (multiplied by `1 / (length || 1)`), stored as f32.
pub fn normalize_normals(normals: &mut [f32]) {
    for i in 0..normals.len() / 3 {
        let n = crate::primitives::normalize(at(normals, i));
        set(normals, i, n);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn face_normals_follow_counter_clockwise_winding() {
        let p = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let n = compute_vertex_normals(&p, None);
        assert_eq!(&n[..3], &[0.0, 0.0, 1.0]);
        let n = compute_vertex_normals(&p, Some(&[0, 1, 2]));
        assert_eq!(&n[6..], &[0.0, 0.0, 1.0]);
    }

    #[test]
    fn zero_normals_stay_zero() {
        let mut n = [0.0f32; 3];
        normalize_normals(&mut n);
        assert_eq!(n, [0.0; 3]);
    }
}
