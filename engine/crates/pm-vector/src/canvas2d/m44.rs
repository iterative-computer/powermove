//! `SkM44` as `SkCanvas` composes it: column-major `f32`, with the
//! `preConcat`/`preTranslate`/`preScale` operation order the pinned Skia uses.
//!
//! Portions are derived from Skia, Copyright Google LLC, used under the BSD
//! 3-Clause license.

use super::skpath::Mat;

/// A 4×4 matrix, column-major (`m[col * 4 + row]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct M44(pub [f32; 16]);

impl Default for M44 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl M44 {
    pub const IDENTITY: M44 = M44([1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]);

    /// `SkM44(m0, m4, m8, m12, m1, ...)`: the row-major constructor.
    #[allow(clippy::too_many_arguments)]
    pub fn row_major(
        m0: f32, m4: f32, m8: f32, m12: f32,
        m1: f32, m5: f32, m9: f32, m13: f32,
        m2: f32, m6: f32, m10: f32, m14: f32,
        m3: f32, m7: f32, m11: f32, m15: f32,
    ) -> Self {
        M44([m0, m1, m2, m3, m4, m5, m6, m7, m8, m9, m10, m11, m12, m13, m14, m15])
    }

    /// `rc(r, c)`.
    #[inline]
    pub fn rc(&self, r: usize, c: usize) -> f32 {
        self.0[c * 4 + r]
    }

    /// An affine `[a b c d e f]` (`SkMatrix` order: `a = sx, b = ky, c = kx, d
    /// = sy, e = tx, f = ty`) as an `SkM44`.
    pub fn affine(a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> Self {
        M44([a, b, 0.0, 0.0, c, d, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, e, f, 0.0, 1.0])
    }

    /// `SkM44::setConcat(a, b)`: each column `c0*r0 + (c1*r1 + (c2*r2 + c3*r3))`.
    pub fn concat(a: &M44, b: &M44) -> M44 {
        let mut out = [0.0f32; 16];
        for col in 0..4 {
            let r = &b.0[col * 4..col * 4 + 4];
            for i in 0..4 {
                out[col * 4 + i] = a.0[i] * r[0] + (a.0[4 + i] * r[1] + (a.0[8 + i] * r[2] + a.0[12 + i] * r[3]));
            }
        }
        M44(out)
    }

    /// `preConcat(m)`.
    pub fn pre_concat(&mut self, m: &M44) {
        *self = M44::concat(self, m);
    }

    /// `preTranslate(x, y)`.
    pub fn pre_translate(&mut self, x: f32, y: f32) {
        let k = &mut self.0;
        for i in 0..4 {
            // c0*x + (c1*y + (c2*z + c3)) with z = 0.
            k[12 + i] = k[i] * x + (k[4 + i] * y + (k[8 + i] * 0.0 + k[12 + i]));
        }
    }

    /// `preScale(x, y)`.
    pub fn pre_scale(&mut self, x: f32, y: f32) {
        let k = &mut self.0;
        for i in 0..4 {
            k[i] *= x;
            k[4 + i] *= y;
        }
    }

    /// `SkM44::RectToRect(src, dst)`.
    pub fn rect_to_rect(src: [f32; 4], dst: [f32; 4]) -> M44 {
        let (sw, sh) = (src[2] - src[0], src[3] - src[1]);
        if sw <= 0.0 || sh <= 0.0 {
            return M44::IDENTITY;
        }
        let (dw, dh) = (dst[2] - dst[0], dst[3] - dst[1]);
        if dw <= 0.0 || dh <= 0.0 {
            return M44([0.0; 16]);
        }
        let sx = dw / sw;
        let sy = dh / sh;
        let tx = dst[0] - sx * src[0];
        let ty = dst[1] - sy * src[1];
        M44::affine(sx, 0.0, 0.0, sy, tx, ty)
    }

    /// The 2D part (`asM33` restricted to the affine entries).
    pub fn to_mat(&self) -> Mat {
        Mat {
            sx: self.rc(0, 0),
            kx: self.rc(0, 1),
            tx: self.rc(0, 3),
            ky: self.rc(1, 0),
            sy: self.rc(1, 1),
            ty: self.rc(1, 3),
        }
    }

    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }
}
