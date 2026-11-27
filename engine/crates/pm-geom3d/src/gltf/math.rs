//! three.js r180's `Matrix4`, `Quaternion`, and `Vector3` operations in f64,
//! in their operation order, so poses, bounds, and bone matrices equal the
//! values three.js computes (the same formulas as `pm_render::three::math`,
//! which depends on this crate and not the other way round).

/// A column-major 4 × 4 matrix (`Matrix4.elements`).
pub type Mat4 = [f64; 16];
pub type Vec3 = [f64; 3];
/// `[x, y, z, w]`.
pub type Quat = [f64; 4];

pub const IDENTITY: Mat4 = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// `Matrix4.compose(position, quaternion, scale)`.
pub fn compose(position: Vec3, q: [f64; 4], scale: Vec3) -> Mat4 {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let (x2, y2, z2) = (x + x, y + y, z + z);
    let (xx, xy, xz) = (x * x2, x * y2, x * z2);
    let (yy, yz, zz) = (y * y2, y * z2, z * z2);
    let (wx, wy, wz) = (w * x2, w * y2, w * z2);
    let (sx, sy, sz) = (scale[0], scale[1], scale[2]);
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
        position[0],
        position[1],
        position[2],
        1.0,
    ]
}

/// `Matrix4.multiplyMatrices(a, b)`.
pub fn multiply(a: &Mat4, b: &Mat4) -> Mat4 {
    let (a11, a12, a13, a14) = (a[0], a[4], a[8], a[12]);
    let (a21, a22, a23, a24) = (a[1], a[5], a[9], a[13]);
    let (a31, a32, a33, a34) = (a[2], a[6], a[10], a[14]);
    let (a41, a42, a43, a44) = (a[3], a[7], a[11], a[15]);
    let (b11, b12, b13, b14) = (b[0], b[4], b[8], b[12]);
    let (b21, b22, b23, b24) = (b[1], b[5], b[9], b[13]);
    let (b31, b32, b33, b34) = (b[2], b[6], b[10], b[14]);
    let (b41, b42, b43, b44) = (b[3], b[7], b[11], b[15]);
    let mut t = [0.0; 16];
    t[0] = a11 * b11 + a12 * b21 + a13 * b31 + a14 * b41;
    t[4] = a11 * b12 + a12 * b22 + a13 * b32 + a14 * b42;
    t[8] = a11 * b13 + a12 * b23 + a13 * b33 + a14 * b43;
    t[12] = a11 * b14 + a12 * b24 + a13 * b34 + a14 * b44;
    t[1] = a21 * b11 + a22 * b21 + a23 * b31 + a24 * b41;
    t[5] = a21 * b12 + a22 * b22 + a23 * b32 + a24 * b42;
    t[9] = a21 * b13 + a22 * b23 + a23 * b33 + a24 * b43;
    t[13] = a21 * b14 + a22 * b24 + a23 * b34 + a24 * b44;
    t[2] = a31 * b11 + a32 * b21 + a33 * b31 + a34 * b41;
    t[6] = a31 * b12 + a32 * b22 + a33 * b32 + a34 * b42;
    t[10] = a31 * b13 + a32 * b23 + a33 * b33 + a34 * b43;
    t[14] = a31 * b14 + a32 * b24 + a33 * b34 + a34 * b44;
    t[3] = a41 * b11 + a42 * b21 + a43 * b31 + a44 * b41;
    t[7] = a41 * b12 + a42 * b22 + a43 * b32 + a44 * b42;
    t[11] = a41 * b13 + a42 * b23 + a43 * b33 + a44 * b43;
    t[15] = a41 * b14 + a42 * b24 + a43 * b34 + a44 * b44;
    t
}

/// `Matrix4.invert()` (all zeros for a singular matrix).
#[allow(clippy::many_single_char_names)]
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

/// `Matrix4.determinant()`.
pub fn determinant(te: &Mat4) -> f64 {
    let (n11, n12, n13, n14) = (te[0], te[4], te[8], te[12]);
    let (n21, n22, n23, n24) = (te[1], te[5], te[9], te[13]);
    let (n31, n32, n33, n34) = (te[2], te[6], te[10], te[14]);
    let (n41, n42, n43, n44) = (te[3], te[7], te[11], te[15]);
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

/// `Quaternion.setFromRotationMatrix(m)`: `[x, y, z, w]`.
pub fn quaternion_from_rotation(m: &Mat4) -> [f64; 4] {
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

/// `Matrix4.decompose(position, quaternion, scale)`.
pub fn decompose(te: &Mat4) -> (Vec3, Quat, Vec3) {
    let len = |a: f64, b: f64, c: f64| (a * a + b * b + c * c).sqrt();
    let mut sx = len(te[0], te[1], te[2]);
    let sy = len(te[4], te[5], te[6]);
    let sz = len(te[8], te[9], te[10]);
    if determinant(te) < 0.0 {
        sx = -sx;
    }
    let mut m = *te;
    let (ix, iy, iz) = (1.0 / sx, 1.0 / sy, 1.0 / sz);
    for k in 0..3 {
        m[k] *= ix;
        m[4 + k] *= iy;
        m[8 + k] *= iz;
    }
    (
        [te[12], te[13], te[14]],
        quaternion_from_rotation(&m),
        [sx, sy, sz],
    )
}

/// `Vector3.applyMatrix4(m)` (with the perspective divide).
pub fn apply(m: &Mat4, v: Vec3) -> Vec3 {
    let (x, y, z) = (v[0], v[1], v[2]);
    let w = 1.0 / (m[3] * x + m[7] * y + m[11] * z + m[15]);
    [
        (m[0] * x + m[4] * y + m[8] * z + m[12]) * w,
        (m[1] * x + m[5] * y + m[9] * z + m[13]) * w,
        (m[2] * x + m[6] * y + m[10] * z + m[14]) * w,
    ]
}

/// `Matrix4.getMaxScaleOnAxis()`.
pub fn max_scale_on_axis(te: &Mat4) -> f64 {
    let sx = te[0] * te[0] + te[1] * te[1] + te[2] * te[2];
    let sy = te[4] * te[4] + te[5] * te[5] + te[6] * te[6];
    let sz = te[8] * te[8] + te[9] * te[9] + te[10] * te[10];
    sx.max(sy).max(sz).sqrt()
}

/// `Matrix4.makeScale(s, s, s)`.
pub fn scale(s: f64) -> Mat4 {
    let mut m = IDENTITY;
    m[0] = s;
    m[5] = s;
    m[10] = s;
    m
}

/// `Matrix4.makeTranslation(x, y, z)`.
pub fn translation(t: Vec3) -> Mat4 {
    let mut m = IDENTITY;
    m[12] = t[0];
    m[13] = t[1];
    m[14] = t[2];
    m
}

/// `Quaternion.normalize()`.
pub fn normalize_quat(q: Quat) -> Quat {
    let l = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if l == 0.0 {
        [0.0, 0.0, 0.0, 1.0]
    } else {
        let inv = 1.0 / l;
        [q[0] * inv, q[1] * inv, q[2] * inv, q[3] * inv]
    }
}

/// A `Float32Array` element: the value WebGL and three.js's typed arrays
/// hold.
pub fn f32_of(v: f64) -> f64 {
    f64::from(v as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_decompose_round_trip() {
        let q = normalize_quat([0.1, 0.7, -0.2, 0.6]);
        let m = compose([1.0, -2.0, 3.0], q, [2.0, 0.5, -1.5]);
        let (t, r, s) = decompose(&m);
        let back = compose(t, r, s);
        for (a, b) in m.iter().zip(back.iter()) {
            assert!((a - b).abs() < 1e-12, "{a} {b}");
        }
        assert!(s[0] < 0.0, "a mirrored matrix negates x scale");
    }

    #[test]
    fn invert_is_inverse() {
        let m = compose(
            [1.0, 2.0, 3.0],
            normalize_quat([0.3, 0.1, 0.2, 0.9]),
            [1.5, 2.0, 0.5],
        );
        let p = multiply(&m, &invert(&m));
        for (k, v) in p.iter().enumerate() {
            let e = if k % 5 == 0 { 1.0 } else { 0.0 };
            assert!((v - e).abs() < 1e-12);
        }
    }
}
