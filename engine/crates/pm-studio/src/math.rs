//! The small vector type scene preparation needs. The GPU does the real math.

use std::ops::{Add, Div, Index, Mul, Neg, Sub};

// Stable, packed RGB triples for host image transfers and exact cache keys.
#[repr(C)]
#[derive(
    Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize,
    bytemuck::Pod, bytemuck::Zeroable,
)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub const fn v3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

impl Vec3 {
    pub const ZERO: Vec3 = v3(0.0, 0.0, 0.0);
    pub const Y: Vec3 = v3(0.0, 1.0, 0.0);
    pub fn splat(v: f32) -> Vec3 {
        v3(v, v, v)
    }
    pub fn dot(self, o: Vec3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn cross(self, o: Vec3) -> Vec3 {
        v3(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }
    pub fn normalize(self) -> Vec3 {
        if self.to_array().iter().any(|v| !v.is_finite()) { return Self::ZERO; }
        let scale = self.x.abs().max(self.y.abs()).max(self.z.abs());
        if scale == 0. { return Self::ZERO; }
        let scaled = self / scale;
        scaled / scaled.length()
    }
    pub fn min(self, o: Vec3) -> Vec3 {
        v3(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }
    pub fn max(self, o: Vec3) -> Vec3 {
        v3(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
    }
    pub fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }
        pub fn to4(self, w: f32) -> [f32; 4] {
        [self.x, self.y, self.z, w]
    }
    /// Rotate about +y by `radians` (turntables).
    pub fn rotate_y(self, radians: f32) -> Vec3 {
        let (s, c) = radians.sin_cos();
        v3(c * self.x + s * self.z, self.y, -s * self.x + c * self.z)
    }
    /// Rotate about +x by `radians` (tilts).
    pub fn rotate_x(self, radians: f32) -> Vec3 {
        let (s, c) = radians.sin_cos();
        v3(self.x, c * self.y - s * self.z, s * self.y + c * self.z)
    }
}

impl Index<usize> for Vec3 {
    type Output = f32;
    fn index(&self, i: usize) -> &f32 {
        match i {
            0 => &self.x,
            1 => &self.y,
            _ => &self.z,
        }
    }
}
impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        v3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        v3(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl Mul<f32> for Vec3 {
    type Output = Vec3;
    fn mul(self, s: f32) -> Vec3 {
        v3(self.x * s, self.y * s, self.z * s)
    }
}
impl Div<f32> for Vec3 {
    type Output = Vec3;
    fn div(self, s: f32) -> Vec3 {
        v3(self.x / s, self.y / s, self.z / s)
    }
}
impl Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        v3(-self.x, -self.y, -self.z)
    }
}

/// An sRGB hex color (`#F6F5F8`) as linear RGB.
pub fn srgb_hex(hex: &str) -> Vec3 {
    let h = hex.strip_prefix('#').unwrap_or(hex);
    let rgb = if h.len() == 6 { u32::from_str_radix(h,16).unwrap_or(0) } else { 0 };
    let c = |shift: u32| ((rgb >> shift) & 255) as f32 / 255.;
    let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    v3(lin(c(16)), lin(c(8)), lin(c(0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalization_handles_extreme_and_degenerate_vectors() {
        assert_eq!(v3(0.,0.,0.).normalize(),Vec3::ZERO);
        assert_eq!(v3(f32::NAN,0.,0.).normalize(),Vec3::ZERO);
        for scale in [1e-30,1e30] {
            let n=v3(scale,scale,0.).normalize();
            assert!((n.length()-1.).abs()<1e-6);
            assert!((n.x-std::f32::consts::FRAC_1_SQRT_2).abs()<1e-6);
        }
    }
    #[test]
    fn malformed_color_strings_never_panic() {
        for value in ["", "#", "#123", "#é1234", "#🙂00", "#GGGGGG"] {
            assert_eq!(srgb_hex(value),Vec3::ZERO);
        }
        assert_eq!(srgb_hex("#FF0000"),v3(1.,0.,0.));
    }
}
