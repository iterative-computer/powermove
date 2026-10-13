//! three.js r180's math, ported where the 3D pass needs its exact numbers:
//! `Matrix4` (`lookAt`, `compose`, `invert`, `multiplyMatrices`,
//! `makePerspective`, `makeOrthographic`), `Quaternion.setFromRotationMatrix`,
//! `Matrix3.getNormalMatrix`, `Vector3.transformDirection` and `normalize`,
//! and `Color.setHex` with sRGB to linear conversion.
//!
//! Every function keeps three.js's operation order in f64; uniforms are then
//! converted to f32 as `Float32Array` does, so the values the GPU receives
//! equal three.js's (checked against recorded draws by `tests/scene3d.rs`).
//! Matrices are column-major (`elements`).

pub type Mat4 = [f64; 16];
pub type Mat3 = [f64; 9];
pub type Vec3 = [f64; 3];

pub const IDENTITY: Mat4 = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

fn length_sq(v: Vec3) -> f64 {
    v[0] * v[0] + v[1] * v[1] + v[2] * v[2]
}

/// `Vector3.normalize()`: `divideScalar(length() || 1)` (a multiply by the
/// reciprocal).
pub fn normalize(v: Vec3) -> Vec3 {
    let l = length_sq(v).sqrt();
    let l = if l == 0.0 || l.is_nan() { 1.0 } else { l };
    let s = 1.0 / l;
    [v[0] * s, v[1] * s, v[2] * s]
}

/// `Vector3.crossVectors(a, b)`.
pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// `Matrix4.lookAt(eye, target, up)`: the rotation part only (the rest of
/// `m` is kept, as three.js does).
pub fn look_at(m: &mut Mat4, eye: Vec3, target: Vec3, up: Vec3) {
    let mut z = [eye[0] - target[0], eye[1] - target[1], eye[2] - target[2]];
    if length_sq(z) == 0.0 {
        z[2] = 1.0;
    }
    z = normalize(z);
    let mut x = cross(up, z);
    if length_sq(x) == 0.0 {
        if up[2].abs() == 1.0 {
            z[0] += 0.0001;
        } else {
            z[2] += 0.0001;
        }
        z = normalize(z);
        x = cross(up, z);
    }
    x = normalize(x);
    let y = cross(z, x);
    m[0] = x[0];
    m[4] = y[0];
    m[8] = z[0];
    m[1] = x[1];
    m[5] = y[1];
    m[9] = z[1];
    m[2] = x[2];
    m[6] = y[2];
    m[10] = z[2];
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
    let t11 = n23 * n34 * n42 - n24 * n33 * n42 + n24 * n32 * n43 - n22 * n34 * n43
        - n23 * n32 * n44
        + n22 * n33 * n44;
    let t12 = n14 * n33 * n42 - n13 * n34 * n42 - n14 * n32 * n43 + n12 * n34 * n43
        + n13 * n32 * n44
        - n12 * n33 * n44;
    let t13 = n13 * n24 * n42 - n14 * n23 * n42 + n14 * n22 * n43 - n12 * n24 * n43
        - n13 * n22 * n44
        + n12 * n23 * n44;
    let t14 = n14 * n23 * n32 - n13 * n24 * n32 - n14 * n22 * n33 + n12 * n24 * n33
        + n13 * n22 * n34
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
    n41 * (n14 * n23 * n32 - n13 * n24 * n32 - n14 * n22 * n33
        + n12 * n24 * n33
        + n13 * n22 * n34
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

/// `Matrix3.getNormalMatrix(m)`: `setFromMatrix4(m).invert().transpose()`.
pub fn normal_matrix(m: &Mat4) -> Mat3 {
    let te = [m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10]];
    let (n11, n21, n31) = (te[0], te[1], te[2]);
    let (n12, n22, n32) = (te[3], te[4], te[5]);
    let (n13, n23, n33) = (te[6], te[7], te[8]);
    let t11 = n33 * n22 - n32 * n23;
    let t12 = n32 * n13 - n33 * n12;
    let t13 = n23 * n12 - n22 * n13;
    let det = n11 * t11 + n21 * t12 + n31 * t13;
    let inv = if det == 0.0 {
        [0.0; 9]
    } else {
        let d = 1.0 / det;
        [
            t11 * d,
            (n31 * n23 - n33 * n21) * d,
            (n32 * n21 - n31 * n22) * d,
            t12 * d,
            (n33 * n11 - n31 * n13) * d,
            (n31 * n12 - n32 * n11) * d,
            t13 * d,
            (n21 * n13 - n23 * n11) * d,
            (n22 * n11 - n21 * n12) * d,
        ]
    };
    // transpose()
    [
        inv[0], inv[3], inv[6], inv[1], inv[4], inv[7], inv[2], inv[5], inv[8],
    ]
}

/// `Vector3.transformDirection(m)` (normalized).
pub fn transform_direction(v: Vec3, e: &Mat4) -> Vec3 {
    let (x, y, z) = (v[0], v[1], v[2]);
    normalize([
        e[0] * x + e[4] * y + e[8] * z,
        e[1] * x + e[5] * y + e[9] * z,
        e[2] * x + e[6] * y + e[10] * z,
    ])
}

/// `Matrix4.makePerspective` (WebGL clip space).
pub fn make_perspective(left: f64, right: f64, top: f64, bottom: f64, near: f64, far: f64) -> Mat4 {
    let x = 2.0 * near / (right - left);
    let y = 2.0 * near / (top - bottom);
    let a = (right + left) / (right - left);
    let b = (top + bottom) / (top - bottom);
    let c = -(far + near) / (far - near);
    let d = (-2.0 * far * near) / (far - near);
    [
        x, 0.0, 0.0, 0.0, 0.0, y, 0.0, 0.0, a, b, c, -1.0, 0.0, 0.0, d, 0.0,
    ]
}

/// `Matrix4.makeOrthographic` (WebGL clip space).
pub fn make_orthographic(
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
    near: f64,
    far: f64,
) -> Mat4 {
    let x = 2.0 / (right - left);
    let y = 2.0 / (top - bottom);
    let a = -(right + left) / (right - left);
    let b = -(top + bottom) / (top - bottom);
    let c = -2.0 / (far - near);
    let d = -(far + near) / (far - near);
    [
        x, 0.0, 0.0, 0.0, 0.0, y, 0.0, 0.0, 0.0, 0.0, c, 0.0, a, b, d, 1.0,
    ]
}

/// `PerspectiveCamera.updateProjectionMatrix()` without a view offset or
/// film offset.
pub fn perspective(fov: f64, aspect: f64, zoom: f64, near: f64, far: f64) -> Mat4 {
    let deg2rad = std::f64::consts::PI / 180.0;
    let top = near * libm::tan(deg2rad * 0.5 * fov) / zoom;
    let height = 2.0 * top;
    let width = aspect * height;
    let left = -0.5 * width;
    make_perspective(left, left + width, top, top - height, near, far)
}

/// `OrthographicCamera.updateProjectionMatrix()` for the frustum
/// `resizeCamera` sets (`±3 · aspect`, `±3`).
pub fn orthographic(aspect: f64, zoom: f64, near: f64, far: f64) -> Mat4 {
    let (l, r, t, b) = (-3.0 * aspect, 3.0 * aspect, 3.0, -3.0);
    let dx = (r - l) / (2.0 * zoom);
    let dy = (t - b) / (2.0 * zoom);
    let cx = (r + l) / 2.0;
    let cy = (t + b) / 2.0;
    make_orthographic(cx - dx, cx + dx, cy + dy, cy - dy, near, far)
}

/// `SRGBToLinear(c)` as three.js r180 writes it.
pub fn srgb_to_linear(c: f64) -> f64 {
    if c < 0.04045 {
        c * 0.0773993808
    } else {
        libm::pow(c * 0.9478672986 + 0.0521327014, 2.4)
    }
}

/// `LinearToSRGB(c)` as three.js r180 writes it.
pub fn linear_to_srgb(c: f64) -> f64 {
    if c < 0.0031308 {
        c * 12.92
    } else {
        1.055 * libm::pow(c, 0.41666) - 0.055
    }
}

/// `new Color().set('#RRGGBB')`: the working (linear sRGB) components.
/// `None` for anything but six hex digits (the scene schema allows only
/// those).
pub fn color_hex(s: &str) -> Option<[f64; 3]> {
    let b = s.as_bytes();
    if b.len() != 7 || b[0] != b'#' || !b[1..].iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let hex = u32::from_str_radix(&s[1..], 16).ok()?;
    Some([
        srgb_to_linear(f64::from((hex >> 16) & 255) / 255.0),
        srgb_to_linear(f64::from((hex >> 8) & 255) / 255.0),
        srgb_to_linear(f64::from(hex & 255) / 255.0),
    ])
}

/// A camera's `matrixWorld` as `Object3D.lookAt` then `updateMatrixWorld`
/// leave it: the look-at rotation goes through a quaternion and
/// `compose` (scale 1).
pub fn camera_world(position: Vec3, target: Vec3, up: Vec3) -> Mat4 {
    let mut m = IDENTITY;
    look_at(&mut m, position, target, up);
    let q = quaternion_from_rotation(&m);
    compose(position, q, [1.0, 1.0, 1.0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invert_round_trips() {
        let m = camera_world([3.0, 2.0, 6.0], [0.0; 3], [0.0, 1.0, 0.0]);
        let p = multiply(&m, &invert(&m));
        for (i, v) in p.iter().enumerate() {
            let e = if i % 5 == 0 { 1.0 } else { 0.0 };
            assert!((v - e).abs() < 1e-12, "{i}: {v}");
        }
        assert!((determinant(&m) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn colors_decode_like_three() {
        let c = color_hex("#4C8DFF").unwrap();
        // Recorded `diffuse` for #4C8DFF (sphere-corpus). `libm::pow` and
        // V8's differ by at most an ulp of f64; the GPU receives f32.
        let recorded = [0.0722718506743852f64, 0.2663556047920505, 1.0];
        for k in 0..3 {
            assert_eq!(c[k] as f32, recorded[k] as f32);
            assert!((c[k] - recorded[k]).abs() <= 2.0 * f64::EPSILON * recorded[k]);
        }
        assert_eq!(color_hex("#4C8DF"), None);
    }

    #[test]
    fn orthographic_frustum() {
        let p = orthographic(2.0, 1.0, 0.01, 1000.0);
        assert!((p[0] - 1.0 / 6.0).abs() < 1e-15);
        assert!((p[5] - 1.0 / 3.0).abs() < 1e-15);
    }
}
