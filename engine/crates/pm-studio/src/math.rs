//! The small vector type scene preparation needs. The GPU does the real math.

use std::ops::{Add, Div, Index, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
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
        let l = self.length();
        if l > 0.0 { self / l } else { self }
    }
    pub fn min(self, o: Vec3) -> Vec3 {
        v3(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }
    pub fn max(self, o: Vec3) -> Vec3 {
        v3(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
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
    let h = hex.trim_start_matches('#');
    let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0) as f32 / 255.0;
    let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    v3(lin(c(0)), lin(c(2)), lin(c(4)))
}
