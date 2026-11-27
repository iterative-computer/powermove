//! The three.js r180 matrix operations model import needs, in f64 with
//! three.js's operation order: column-major 4 × 4 matrices (`elements`).

pub type Mat4 = [f64; 16];

pub const IDENTITY: Mat4 = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// `Matrix4.compose(position, quaternion, scale)`.
pub fn compose(t: [f64; 3], q: [f64; 4], s: [f64; 3]) -> Mat4 {
    let [x, y, z, w] = q;
    let (x2, y2, z2) = (x + x, y + y, z + z);
    let (xx, xy, xz) = (x * x2, x * y2, x * z2);
    let (yy, yz, zz) = (y * y2, y * z2, z * z2);
    let (wx, wy, wz) = (w * x2, w * y2, w * z2);
    let [sx, sy, sz] = s;
    [
        (1.0 - (yy + zz)) * sx,
        (xy + wz) * sx,
        (xz - wy) * sx,
        0.0,
        (xy - wz) * sy,
        (1.0 - (xx + zz)) * sy,
        (yz + wx) * sy,
        0.0,
        (xz + wy) * sz,
        (yz - wx) * sz,
        (1.0 - (xx + yy)) * sz,
        0.0,
        t[0],
        t[1],
        t[2],
        1.0,
    ]
}

/// `Matrix4.multiplyMatrices(a, b)` (a × b).
pub fn multiply(a: &Mat4, b: &Mat4) -> Mat4 {
    let (a11, a12, a13, a14) = (a[0], a[4], a[8], a[12]);
    let (a21, a22, a23, a24) = (a[1], a[5], a[9], a[13]);
    let (a31, a32, a33, a34) = (a[2], a[6], a[10], a[14]);
    let (a41, a42, a43, a44) = (a[3], a[7], a[11], a[15]);
    let (b11, b12, b13, b14) = (b[0], b[4], b[8], b[12]);
    let (b21, b22, b23, b24) = (b[1], b[5], b[9], b[13]);
    let (b31, b32, b33, b34) = (b[2], b[6], b[10], b[14]);
    let (b41, b42, b43, b44) = (b[3], b[7], b[11], b[15]);
    let mut te = [0.0; 16];
    te[0] = a11 * b11 + a12 * b21 + a13 * b31 + a14 * b41;
    te[4] = a11 * b12 + a12 * b22 + a13 * b32 + a14 * b42;
    te[8] = a11 * b13 + a12 * b23 + a13 * b33 + a14 * b43;
    te[12] = a11 * b14 + a12 * b24 + a13 * b34 + a14 * b44;
    te[1] = a21 * b11 + a22 * b21 + a23 * b31 + a24 * b41;
    te[5] = a21 * b12 + a22 * b22 + a23 * b32 + a24 * b42;
    te[9] = a21 * b13 + a22 * b23 + a23 * b33 + a24 * b43;
    te[13] = a21 * b14 + a22 * b24 + a23 * b34 + a24 * b44;
    te[2] = a31 * b11 + a32 * b21 + a33 * b31 + a34 * b41;
    te[6] = a31 * b12 + a32 * b22 + a33 * b32 + a34 * b42;
    te[10] = a31 * b13 + a32 * b23 + a33 * b33 + a34 * b43;
    te[14] = a31 * b14 + a32 * b24 + a33 * b34 + a34 * b44;
    te[3] = a41 * b11 + a42 * b21 + a43 * b31 + a44 * b41;
    te[7] = a41 * b12 + a42 * b22 + a43 * b32 + a44 * b42;
    te[11] = a41 * b13 + a42 * b23 + a43 * b33 + a44 * b43;
    te[15] = a41 * b14 + a42 * b24 + a43 * b34 + a44 * b44;
    te
}

/// `Vector3.applyMatrix4(m)` (with the perspective divide).
pub fn apply(m: &Mat4, v: [f64; 3]) -> [f64; 3] {
    let [x, y, z] = v;
    let w = 1.0 / (m[3] * x + m[7] * y + m[11] * z + m[15]);
    [
        (m[0] * x + m[4] * y + m[8] * z + m[12]) * w,
        (m[1] * x + m[5] * y + m[9] * z + m[13]) * w,
        (m[2] * x + m[6] * y + m[10] * z + m[14]) * w,
    ]
}

/// `Matrix4.decompose()`: translation, quaternion, scale.
pub fn decompose(m: &Mat4) -> ([f64; 3], [f64; 4], [f64; 3]) {
    let len = |x: f64, y: f64, z: f64| (x * x + y * y + z * z).sqrt();
    let mut sx = len(m[0], m[1], m[2]);
    let sy = len(m[4], m[5], m[6]);
    let sz = len(m[8], m[9], m[10]);
    if determinant(m) < 0.0 {
        sx = -sx;
    }
    let t = [m[12], m[13], m[14]];
    let mut r = *m;
    let (ix, iy, iz) = (1.0 / sx, 1.0 / sy, 1.0 / sz);
    r[0] *= ix;
    r[1] *= ix;
    r[2] *= ix;
    r[4] *= iy;
    r[5] *= iy;
    r[6] *= iy;
    r[8] *= iz;
    r[9] *= iz;
    r[10] *= iz;
    (t, quaternion_from_rotation(&r), [sx, sy, sz])
}

/// `Matrix4.determinant()`.
pub fn determinant(m: &Mat4) -> f64 {
    let (n11, n12, n13, n14) = (m[0], m[4], m[8], m[12]);
    let (n21, n22, n23, n24) = (m[1], m[5], m[9], m[13]);
    let (n31, n32, n33, n34) = (m[2], m[6], m[10], m[14]);
    let (n41, n42, n43, n44) = (m[3], m[7], m[11], m[15]);
    n41 * (n14 * n23 * n32 - n13 * n24 * n32 - n14 * n22 * n33 + n12 * n24 * n33 + n13 * n22 * n34
        - n12 * n23 * n34)
        + n42
            * (n11 * n23 * n34 - n11 * n24 * n33 + n14 * n21 * n33 - n13 * n21 * n34
                + n13 * n24 * n31
                - n14 * n23 * n31)
        + n43
            * (n11 * n24 * n32 - n11 * n22 * n34 - n14 * n21 * n32
                + n12 * n21 * n34
                + n14 * n22 * n31
                - n12 * n24 * n31)
        + n44
            * (-n13 * n22 * n31 - n11 * n23 * n32 + n11 * n22 * n33 + n13 * n21 * n32
                - n12 * n21 * n33
                + n12 * n23 * n31)
}

/// `Matrix4.invert()` (all zeros when singular, as three.js returns).
pub fn invert(te: &Mat4) -> Mat4 {
    let (n11, n21, n31, n41) = (te[0], te[1], te[2], te[3]);
    let (n12, n22, n32, n42) = (te[4], te[5], te[6], te[7]);
    let (n13, n23, n33, n43) = (te[8], te[9], te[10], te[11]);
    let (n14, n24, n34, n44) = (te[12], te[13], te[14], te[15]);
    let t11 =
        n23 * n34 * n42 - n24 * n33 * n42 + n24 * n32 * n43 - n22 * n34 * n43 - n23 * n32 * n44
            + n22 * n33 * n44;
    let t12 =
        n14 * n33 * n42 - n13 * n34 * n42 - n14 * n32 * n43 + n12 * n34 * n43 + n13 * n32 * n44
            - n12 * n33 * n44;
    let t13 =
        n13 * n24 * n42 - n14 * n23 * n42 + n14 * n22 * n43 - n12 * n24 * n43 - n13 * n22 * n44
            + n12 * n23 * n44;
    let t14 =
        n14 * n23 * n32 - n13 * n24 * n32 - n14 * n22 * n33 + n12 * n24 * n33 + n13 * n22 * n34
            - n12 * n23 * n34;
    let det = n11 * t11 + n21 * t12 + n31 * t13 + n41 * t14;
    if det == 0.0 {
        return [0.0; 16];
    }
    let d = 1.0 / det;
    [
        t11 * d,
        (n24 * n33 * n41 - n23 * n34 * n41 - n24 * n31 * n43 + n21 * n34 * n43 + n23 * n31 * n44
            - n21 * n33 * n44)
            * d,
        (n22 * n34 * n41 - n24 * n32 * n41 + n24 * n31 * n42 - n21 * n34 * n42 - n22 * n31 * n44
            + n21 * n32 * n44)
            * d,
        (n23 * n32 * n41 - n22 * n33 * n41 - n23 * n31 * n42 + n21 * n33 * n42 + n22 * n31 * n43
            - n21 * n32 * n43)
            * d,
        t12 * d,
        (n13 * n34 * n41 - n14 * n33 * n41 + n14 * n31 * n43 - n11 * n34 * n43 - n13 * n31 * n44
            + n11 * n33 * n44)
            * d,
        (n14 * n32 * n41 - n12 * n34 * n41 - n14 * n31 * n42 + n11 * n34 * n42 + n12 * n31 * n44
            - n11 * n32 * n44)
            * d,
        (n12 * n33 * n41 - n13 * n32 * n41 + n13 * n31 * n42 - n11 * n33 * n42 - n12 * n31 * n43
            + n11 * n32 * n43)
            * d,
        t13 * d,
        (n14 * n23 * n41 - n13 * n24 * n41 - n14 * n21 * n43 + n11 * n24 * n43 + n13 * n21 * n44
            - n11 * n23 * n44)
            * d,
        (n12 * n24 * n41 - n14 * n22 * n41 + n14 * n21 * n42 - n11 * n24 * n42 - n12 * n21 * n44
            + n11 * n22 * n44)
            * d,
        (n13 * n22 * n41 - n12 * n23 * n41 - n13 * n21 * n42 + n11 * n23 * n42 + n12 * n21 * n43
            - n11 * n22 * n43)
            * d,
        t14 * d,
        (n13 * n24 * n31 - n14 * n23 * n31 + n14 * n21 * n33 - n11 * n24 * n33 - n13 * n21 * n34
            + n11 * n23 * n34)
            * d,
        (n14 * n22 * n31 - n12 * n24 * n31 - n14 * n21 * n32 + n11 * n24 * n32 + n12 * n21 * n34
            - n11 * n22 * n34)
            * d,
        (n12 * n23 * n31 - n13 * n22 * n31 + n13 * n21 * n32 - n11 * n23 * n32 - n12 * n21 * n33
            + n11 * n22 * n33)
            * d,
    ]
}

/// `Quaternion.setFromRotationMatrix(m)`.
fn quaternion_from_rotation(m: &Mat4) -> [f64; 4] {
    let (m11, m12, m13) = (m[0], m[4], m[8]);
    let (m21, m22, m23) = (m[1], m[5], m[9]);
    let (m31, m32, m33) = (m[2], m[6], m[10]);
    let trace = m11 + m22 + m33;
    if trace > 0.0 {
        let s = 0.5 / (trace + 1.0).sqrt();
        [(m32 - m23) * s, (m13 - m31) * s, (m21 - m12) * s, 0.25 / s]
    } else if m11 > m22 && m11 > m33 {
        let s = 2.0 * (1.0 + m11 - m22 - m33).sqrt();
        [0.25 * s, (m12 + m21) / s, (m13 + m31) / s, (m32 - m23) / s]
    } else if m22 > m33 {
        let s = 2.0 * (1.0 + m22 - m11 - m33).sqrt();
        [(m12 + m21) / s, 0.25 * s, (m23 + m32) / s, (m13 - m31) / s]
    } else {
        let s = 2.0 * (1.0 + m33 - m11 - m22).sqrt();
        [(m13 + m31) / s, (m23 + m32) / s, 0.25 * s, (m21 - m12) / s]
    }
}

/// An axis-aligned box (`Box3`), empty as three.js makes it empty.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Box3 {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Default for Box3 {
    fn default() -> Self {
        Box3 {
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
        }
    }
}

impl Box3 {
    pub fn is_empty(&self) -> bool {
        self.max[0] < self.min[0] || self.max[1] < self.min[1] || self.max[2] < self.min[2]
    }

    pub fn expand_by_point(&mut self, p: [f64; 3]) {
        for (c, v) in p.into_iter().enumerate() {
            self.min[c] = self.min[c].min(v);
            self.max[c] = self.max[c].max(v);
        }
    }

    pub fn union(&mut self, b: &Box3) {
        for c in 0..3 {
            self.min[c] = self.min[c].min(b.min[c]);
            self.max[c] = self.max[c].max(b.max[c]);
        }
    }

    /// `Box3.applyMatrix4(m)`: the box of its 8 transformed corners.
    pub fn apply_matrix(&self, m: &Mat4) -> Box3 {
        if self.is_empty() {
            return *self;
        }
        let (lo, hi) = (self.min, self.max);
        let mut out = Box3::default();
        for &x in &[lo[0], hi[0]] {
            for &y in &[lo[1], hi[1]] {
                for &z in &[lo[2], hi[2]] {
                    out.expand_by_point(apply(m, [x, y, z]));
                }
            }
        }
        out
    }

    /// `getCenter` (three.js: `(min + max) · 0.5`).
    pub fn center(&self) -> [f64; 3] {
        std::array::from_fn(|c| (self.min[c] + self.max[c]) * 0.5)
    }

    /// `getSize(...).length()`.
    pub fn diagonal(&self) -> f64 {
        let s: [f64; 3] = std::array::from_fn(|c| self.max[c] - self.min[c]);
        (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_and_decompose_round_trip() {
        let q = {
            let (s, c) = (0.3f64.sin(), 0.3f64.cos());
            [0.0, s, 0.0, c]
        };
        let m = compose([1.0, 2.0, 3.0], q, [2.0, 3.0, 4.0]);
        let (t, r, s) = decompose(&m);
        assert_eq!(t, [1.0, 2.0, 3.0]);
        for c in 0..4 {
            assert!((r[c] - q[c]).abs() < 1e-12);
        }
        for (a, b) in s.iter().zip([2.0, 3.0, 4.0]) {
            assert!((a - b).abs() < 1e-12);
        }
        assert_eq!(multiply(&IDENTITY, &m), m);
        let back = multiply(&m, &invert(&m));
        for (a, b) in back.iter().zip(IDENTITY) {
            assert!((a - b).abs() < 1e-12);
        }
        assert_eq!(invert(&[0.0; 16]), [0.0; 16]);
        assert_eq!(apply(&m, [0.0; 3]), [1.0, 2.0, 3.0]);
    }

    #[test]
    fn boxes_transform_their_corners() {
        let mut b = Box3::default();
        assert!(b.is_empty());
        b.expand_by_point([-1.0, -1.0, -1.0]);
        b.expand_by_point([1.0, 1.0, 1.0]);
        let q = [
            0.0,
            (std::f64::consts::FRAC_PI_8).sin(),
            0.0,
            (std::f64::consts::FRAC_PI_8).cos(),
        ];
        let r = b.apply_matrix(&compose([0.0; 3], q, [1.0; 3]));
        assert!((r.max[0] - 2f64.sqrt()).abs() < 1e-12);
        assert_eq!(b.center(), [0.0; 3]);
        assert!((b.diagonal() - 12f64.sqrt()).abs() < 1e-12);
    }
}
