//! Skia's analytic anti-aliased path filling (AAA), ported to fill an A8
//! glyph mask exactly as `SkScalerContext::GenerateImageFromPath` does in
//! Chromium's Skia: `SkScan::AntiFillPath` → `AAAFillPath` with the
//! `MaskAdditiveBlitter` (small masks) or `SafeRLEAdditiveBlitter`, over
//! the A8 source-over blitter.
//!
//! Port of `src/core/SkScan_AAAPath.cpp`, `SkAnalyticEdge.cpp`,
//! `SkEdgeBuilder.cpp` (analytic edges, no clipping), `SkTSort.h` and the
//! Y-extrema chops of `SkGeometry.cpp` from Skia m149, in the same 16.16
//! fixed-point and `f32` arithmetic. Only what non-inverse winding fills of
//! paths contained in the mask need. Convex paths (including a collapsed
//! rectangle stroke and COLR layers) use Skia's separate convex edge walk.
//!
//! Skia is Copyright 2006-2016 The Android Open Source Project and Google
//! LLC, used under its BSD-3-Clause license:
//!
//! Redistribution and use in source and binary forms, with or without
//! modification, are permitted provided that the following conditions are
//! met: (1) Redistributions of source code must retain the above copyright
//! notice, this list of conditions and the following disclaimer.
//! (2) Redistributions in binary form must reproduce the above copyright
//! notice, this list of conditions and the following disclaimer in the
//! documentation and/or other materials provided with the distribution.
//! (3) Neither the name of the copyright holder nor the names of its
//! contributors may be used to endorse or promote products derived from
//! this software without specific prior written permission. THIS SOFTWARE
//! IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY
//! EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
//! IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR
//! PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR
//! CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
//! EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
//! PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
//! PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
//! LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
//! NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
//! SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

// A line-for-line port: Skia's control flow and arithmetic are kept as
// written so the two can be diffed, which these style lints would obscure.
#![allow(
    clippy::too_many_arguments,
    clippy::needless_return,
    clippy::nonminimal_bool,
    clippy::implicit_saturating_sub,
    clippy::field_reassign_with_default,
    clippy::manual_range_contains
)]

#[path = "skaaa_convex.rs"]
mod convex;

type Fixed = i32;
type FDot6 = i32;

const FIXED1: Fixed = 1 << 16;
const MAX_S32: i32 = i32::MAX;
const MIN_S32: i32 = -i32::MAX;
const DEFAULT_ACCURACY: i32 = 2;

#[inline]
fn left_shift(x: i32, n: i32) -> i32 {
    ((x as u32) << n) as i32
}
#[inline]
fn fixed_mul(a: Fixed, b: Fixed) -> Fixed {
    ((i64::from(a) * i64::from(b)) >> 16) as i32
}
#[inline]
fn fixed_div(numer: i32, denom: i32) -> Fixed {
    ((i64::from(numer) << 16) / i64::from(denom)).clamp(i64::from(MIN_S32), i64::from(MAX_S32))
        as i32
}
#[inline]
fn fdot6_to_fixed(x: FDot6) -> Fixed {
    left_shift(x, 10)
}
#[inline]
fn fixed_to_fdot6(x: Fixed) -> FDot6 {
    x >> 10
}
#[inline]
fn fdot6_div(a: FDot6, b: FDot6) -> Fixed {
    if i16::try_from(a).is_ok() {
        left_shift(a, 16) / b
    } else {
        fixed_div(a, b)
    }
}
#[inline]
fn fdot6_round(x: FDot6) -> i32 {
    (x + 32) >> 6
}
#[inline]
fn fixed_floor_to_int(x: Fixed) -> i32 {
    x >> 16
}
#[inline]
fn fixed_ceil_to_int(x: Fixed) -> i32 {
    x.wrapping_add(FIXED1 - 1) >> 16
}
#[inline]
fn fixed_round_to_int(x: Fixed) -> i32 {
    x.wrapping_add(1 << 15) >> 16
}
#[inline]
fn fixed_round_to_fixed(x: Fixed) -> Fixed {
    ((x as u32).wrapping_add(1 << 15) & 0xFFFF_0000) as i32
}
#[inline]
fn fixed_ceil_to_fixed(x: Fixed) -> Fixed {
    ((x as u32).wrapping_add((FIXED1 - 1) as u32) & 0xFFFF_0000) as i32
}
#[inline]
fn fixed_floor_to_fixed(x: Fixed) -> Fixed {
    ((x as u32) & 0xFFFF_0000) as i32
}
#[inline]
fn int_to_fixed(n: i32) -> Fixed {
    ((n as u32) << 16) as i32
}
#[inline]
fn sat_add(a: i32, b: i32) -> i32 {
    (i64::from(a) + i64::from(b)).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}
/// `SkScalarToFDot6`: `(int)(x * 64)`, truncating.
#[inline]
fn scalar_to_fdot6(x: f32) -> FDot6 {
    (x * 64.0) as i32
}
#[inline]
fn snap_y(y: Fixed) -> Fixed {
    let accuracy = DEFAULT_ACCURACY;
    ((((y as u32).wrapping_add((FIXED1 >> (accuracy + 1)) as u32)) >> (16 - accuracy))
        << (16 - accuracy)) as i32
}

const INVERSE_TABLE_SIZE: i32 = 1024;

#[rustfmt::skip]
static INVERSE_TABLE: [i32; 1025] = [
    -4096, -4100, -4104, -4108, -4112, -4116, -4120, -4124, -4128, -4132, -4136, -4140,
    -4144, -4148, -4152, -4156, -4161, -4165, -4169, -4173, -4177, -4181, -4185, -4190,
    -4194, -4198, -4202, -4206, -4211, -4215, -4219, -4223, -4228, -4232, -4236, -4240,
    -4245, -4249, -4253, -4258, -4262, -4266, -4271, -4275, -4279, -4284, -4288, -4293,
    -4297, -4301, -4306, -4310, -4315, -4319, -4324, -4328, -4332, -4337, -4341, -4346,
    -4350, -4355, -4359, -4364, -4369, -4373, -4378, -4382, -4387, -4391, -4396, -4401,
    -4405, -4410, -4415, -4419, -4424, -4429, -4433, -4438, -4443, -4447, -4452, -4457,
    -4462, -4466, -4471, -4476, -4481, -4485, -4490, -4495, -4500, -4505, -4510, -4514,
    -4519, -4524, -4529, -4534, -4539, -4544, -4549, -4554, -4559, -4563, -4568, -4573,
    -4578, -4583, -4588, -4593, -4599, -4604, -4609, -4614, -4619, -4624, -4629, -4634,
    -4639, -4644, -4650, -4655, -4660, -4665, -4670, -4675, -4681, -4686, -4691, -4696,
    -4702, -4707, -4712, -4718, -4723, -4728, -4733, -4739, -4744, -4750, -4755, -4760,
    -4766, -4771, -4777, -4782, -4788, -4793, -4798, -4804, -4809, -4815, -4821, -4826,
    -4832, -4837, -4843, -4848, -4854, -4860, -4865, -4871, -4877, -4882, -4888, -4894,
    -4899, -4905, -4911, -4917, -4922, -4928, -4934, -4940, -4946, -4951, -4957, -4963,
    -4969, -4975, -4981, -4987, -4993, -4999, -5005, -5011, -5017, -5023, -5029, -5035,
    -5041, -5047, -5053, -5059, -5065, -5071, -5077, -5084, -5090, -5096, -5102, -5108,
    -5115, -5121, -5127, -5133, -5140, -5146, -5152, -5159, -5165, -5171, -5178, -5184,
    -5190, -5197, -5203, -5210, -5216, -5223, -5229, -5236, -5242, -5249, -5256, -5262,
    -5269, -5275, -5282, -5289, -5295, -5302, -5309, -5315, -5322, -5329, -5336, -5343,
    -5349, -5356, -5363, -5370, -5377, -5384, -5391, -5398, -5405, -5412, -5418, -5426,
    -5433, -5440, -5447, -5454, -5461, -5468, -5475, -5482, -5489, -5497, -5504, -5511,
    -5518, -5526, -5533, -5540, -5548, -5555, -5562, -5570, -5577, -5584, -5592, -5599,
    -5607, -5614, -5622, -5629, -5637, -5645, -5652, -5660, -5667, -5675, -5683, -5691,
    -5698, -5706, -5714, -5722, -5729, -5737, -5745, -5753, -5761, -5769, -5777, -5785,
    -5793, -5801, -5809, -5817, -5825, -5833, -5841, -5849, -5857, -5866, -5874, -5882,
    -5890, -5899, -5907, -5915, -5924, -5932, -5940, -5949, -5957, -5966, -5974, -5983,
    -5991, -6000, -6009, -6017, -6026, -6034, -6043, -6052, -6061, -6069, -6078, -6087,
    -6096, -6105, -6114, -6123, -6132, -6141, -6150, -6159, -6168, -6177, -6186, -6195,
    -6204, -6213, -6223, -6232, -6241, -6250, -6260, -6269, -6278, -6288, -6297, -6307,
    -6316, -6326, -6335, -6345, -6355, -6364, -6374, -6384, -6393, -6403, -6413, -6423,
    -6432, -6442, -6452, -6462, -6472, -6482, -6492, -6502, -6512, -6523, -6533, -6543,
    -6553, -6563, -6574, -6584, -6594, -6605, -6615, -6626, -6636, -6647, -6657, -6668,
    -6678, -6689, -6700, -6710, -6721, -6732, -6743, -6754, -6765, -6775, -6786, -6797,
    -6808, -6820, -6831, -6842, -6853, -6864, -6875, -6887, -6898, -6909, -6921, -6932,
    -6944, -6955, -6967, -6978, -6990, -7002, -7013, -7025, -7037, -7049, -7061, -7073,
    -7084, -7096, -7108, -7121, -7133, -7145, -7157, -7169, -7182, -7194, -7206, -7219,
    -7231, -7244, -7256, -7269, -7281, -7294, -7307, -7319, -7332, -7345, -7358, -7371,
    -7384, -7397, -7410, -7423, -7436, -7449, -7463, -7476, -7489, -7503, -7516, -7530,
    -7543, -7557, -7570, -7584, -7598, -7612, -7626, -7639, -7653, -7667, -7681, -7695,
    -7710, -7724, -7738, -7752, -7767, -7781, -7796, -7810, -7825, -7839, -7854, -7869,
    -7884, -7898, -7913, -7928, -7943, -7958, -7973, -7989, -8004, -8019, -8035, -8050,
    -8065, -8081, -8097, -8112, -8128, -8144, -8160, -8176, -8192, -8208, -8224, -8240,
    -8256, -8272, -8289, -8305, -8322, -8338, -8355, -8371, -8388, -8405, -8422, -8439,
    -8456, -8473, -8490, -8507, -8525, -8542, -8559, -8577, -8594, -8612, -8630, -8648,
    -8665, -8683, -8701, -8719, -8738, -8756, -8774, -8793, -8811, -8830, -8848, -8867,
    -8886, -8905, -8924, -8943, -8962, -8981, -9000, -9020, -9039, -9058, -9078, -9098,
    -9118, -9137, -9157, -9177, -9198, -9218, -9238, -9258, -9279, -9300, -9320, -9341,
    -9362, -9383, -9404, -9425, -9446, -9467, -9489, -9510, -9532, -9554, -9576, -9597,
    -9619, -9642, -9664, -9686, -9709, -9731, -9754, -9776, -9799, -9822, -9845, -9868,
    -9892, -9915, -9939, -9962, -9986, -10010, -10034, -10058, -10082, -10106, -10131, -10155,
    -10180, -10205, -10230, -10255, -10280, -10305, -10330, -10356, -10381, -10407, -10433, -10459,
    -10485, -10512, -10538, -10564, -10591, -10618, -10645, -10672, -10699, -10727, -10754, -10782,
    -10810, -10837, -10866, -10894, -10922, -10951, -10979, -11008, -11037, -11066, -11096, -11125,
    -11155, -11184, -11214, -11244, -11275, -11305, -11335, -11366, -11397, -11428, -11459, -11491,
    -11522, -11554, -11586, -11618, -11650, -11683, -11715, -11748, -11781, -11814, -11848, -11881,
    -11915, -11949, -11983, -12018, -12052, -12087, -12122, -12157, -12192, -12228, -12264, -12300,
    -12336, -12372, -12409, -12446, -12483, -12520, -12557, -12595, -12633, -12671, -12710, -12748,
    -12787, -12826, -12865, -12905, -12945, -12985, -13025, -13066, -13107, -13148, -13189, -13231,
    -13273, -13315, -13357, -13400, -13443, -13486, -13530, -13573, -13617, -13662, -13706, -13751,
    -13797, -13842, -13888, -13934, -13981, -14027, -14074, -14122, -14169, -14217, -14266, -14315,
    -14364, -14413, -14463, -14513, -14563, -14614, -14665, -14716, -14768, -14820, -14873, -14926,
    -14979, -15033, -15087, -15141, -15196, -15252, -15307, -15363, -15420, -15477, -15534, -15592,
    -15650, -15709, -15768, -15827, -15887, -15947, -16008, -16070, -16131, -16194, -16256, -16320,
    -16384, -16448, -16513, -16578, -16644, -16710, -16777, -16844, -16912, -16980, -17050, -17119,
    -17189, -17260, -17331, -17403, -17476, -17549, -17623, -17697, -17772, -17848, -17924, -18001,
    -18078, -18157, -18236, -18315, -18396, -18477, -18558, -18641, -18724, -18808, -18893, -18978,
    -19065, -19152, -19239, -19328, -19418, -19508, -19599, -19691, -19784, -19878, -19972, -20068,
    -20164, -20262, -20360, -20460, -20560, -20661, -20763, -20867, -20971, -21076, -21183, -21290,
    -21399, -21509, -21620, -21732, -21845, -21959, -22075, -22192, -22310, -22429, -22550, -22671,
    -22795, -22919, -23045, -23172, -23301, -23431, -23563, -23696, -23831, -23967, -24105, -24244,
    -24385, -24528, -24672, -24818, -24966, -25115, -25266, -25420, -25575, -25731, -25890, -26051,
    -26214, -26379, -26546, -26715, -26886, -27060, -27235, -27413, -27594, -27776, -27962, -28149,
    -28339, -28532, -28728, -28926, -29127, -29330, -29537, -29746, -29959, -30174, -30393, -30615,
    -30840, -31068, -31300, -31536, -31775, -32017, -32263, -32513, -32768, -33026, -33288, -33554,
    -33825, -34100, -34379, -34663, -34952, -35246, -35544, -35848, -36157, -36472, -36792, -37117,
    -37449, -37786, -38130, -38479, -38836, -39199, -39568, -39945, -40329, -40721, -41120, -41527,
    -41943, -42366, -42799, -43240, -43690, -44150, -44620, -45100, -45590, -46091, -46603, -47127,
    -47662, -48210, -48770, -49344, -49932, -50533, -51150, -51781, -52428, -53092, -53773, -54471,
    -55188, -55924, -56679, -57456, -58254, -59074, -59918, -60787, -61680, -62601, -63550, -64527,
    -65536, -66576, -67650, -68759, -69905, -71089, -72315, -73584, -74898, -76260, -77672, -79137,
    -80659, -82241, -83886, -85598, -87381, -89240, -91180, -93206, -95325, -97541, -99864, -102300,
    -104857, -107546, -110376, -113359, -116508, -119837, -123361, -127100, -131072, -135300, -139810, -144631,
    -149796, -155344, -161319, -167772, -174762, -182361, -190650, -199728, -209715, -220752, -233016, -246723,
    -262144, -279620, -299593, -322638, -349525, -381300, -419430, -466033, -524288, -599186, -699050, -838860,
    -1048576, -1398101, -2097152, -4194304, 0,
];

#[inline]
fn quick_inverse(x: FDot6) -> Fixed {
    let last = INVERSE_TABLE_SIZE as usize;
    if x > 0 {
        -INVERSE_TABLE[last - x as usize]
    } else {
        INVERSE_TABLE[(last as i32 + x) as usize]
    }
}

#[inline]
fn quick_div(a: FDot6, b: FDot6) -> Fixed {
    const MIN_BITS: i32 = 3;
    const MAX_BITS: i32 = 31;
    const MAX_ABS_A: i32 = 1 << (MAX_BITS - (22 - MIN_BITS));
    let abs_a = a.wrapping_abs();
    let abs_b = b.wrapping_abs();
    if abs_b >= (1 << MIN_BITS) && abs_b < INVERSE_TABLE_SIZE && abs_a < MAX_ABS_A {
        return a.wrapping_mul(quick_inverse(b)) >> 6;
    }
    fdot6_div(a, b)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EdgeType {
    Line,
    Quad,
    Cubic,
}

/// `SkAnalyticEdge` and its quadratic and cubic subclasses in one record,
/// linked by indices into the edge arena.
#[derive(Clone, Copy)]
struct Edge {
    next: usize,
    prev: usize,
    x: Fixed,
    dx: Fixed,
    upper_x: Fixed,
    y: Fixed,
    upper_y: Fixed,
    lower_y: Fixed,
    dy: Fixed,
    edge_type: EdgeType,
    curve_count: i8,
    curve_shift: u8,
    winding: i8,
    // Quadratic.
    qx: Fixed,
    qy: Fixed,
    qdx: Fixed,
    qdy: Fixed,
    qddx: Fixed,
    qddy: Fixed,
    qlast_x: Fixed,
    qlast_y: Fixed,
    snapped_x: Fixed,
    snapped_y: Fixed,
    // Cubic.
    cx: Fixed,
    cy: Fixed,
    cdx: Fixed,
    cdy: Fixed,
    cddx: Fixed,
    cddy: Fixed,
    cdddx: Fixed,
    cdddy: Fixed,
    clast_x: Fixed,
    clast_y: Fixed,
    c_snapped_y: Fixed,
    cubic_dshift: u8,
}

const NONE: usize = usize::MAX;

impl Default for Edge {
    fn default() -> Self {
        Edge {
            next: NONE,
            prev: NONE,
            x: 0,
            dx: 0,
            upper_x: 0,
            y: 0,
            upper_y: 0,
            lower_y: 0,
            dy: 0,
            edge_type: EdgeType::Line,
            curve_count: 0,
            curve_shift: 0,
            winding: 1,
            qx: 0,
            qy: 0,
            qdx: 0,
            qdy: 0,
            qddx: 0,
            qddy: 0,
            qlast_x: 0,
            qlast_y: 0,
            snapped_x: 0,
            snapped_y: 0,
            cx: 0,
            cy: 0,
            cdx: 0,
            cdy: 0,
            cddx: 0,
            cddy: 0,
            cdddx: 0,
            cdddy: 0,
            clast_x: 0,
            clast_y: 0,
            c_snapped_y: 0,
            cubic_dshift: 0,
        }
    }
}

impl Edge {
    #[inline]
    fn go_y(&mut self, y: Fixed) {
        if y == self.y.wrapping_add(FIXED1) {
            self.x = self.x.wrapping_add(self.dx);
            self.y = y;
        } else if y != self.y {
            self.x = self
                .upper_x
                .wrapping_add(fixed_mul(self.dx, y.wrapping_sub(self.upper_y)));
            self.y = y;
        }
    }

    #[inline]
    fn go_y_shift(&mut self, y: Fixed, y_shift: i32) {
        self.y = y;
        self.x = self.x.wrapping_add(self.dx >> y_shift);
    }

    fn set_line(&mut self, p0: (f32, f32), p1: (f32, f32)) -> bool {
        let accuracy = DEFAULT_ACCURACY;
        let multiplier = (1 << DEFAULT_ACCURACY) as f32;
        let mut x0 = fdot6_to_fixed(scalar_to_fdot6(p0.0 * multiplier)) >> accuracy;
        let mut y0 = snap_y(fdot6_to_fixed(scalar_to_fdot6(p0.1 * multiplier)) >> accuracy);
        let mut x1 = fdot6_to_fixed(scalar_to_fdot6(p1.0 * multiplier)) >> accuracy;
        let mut y1 = snap_y(fdot6_to_fixed(scalar_to_fdot6(p1.1 * multiplier)) >> accuracy);
        let mut winding = 1i8;
        if y0 > y1 {
            std::mem::swap(&mut x0, &mut x1);
            std::mem::swap(&mut y0, &mut y1);
            winding = -1;
        }
        let dy = fixed_to_fdot6(y1.wrapping_sub(y0));
        if dy == 0 {
            return false;
        }
        let dx = fixed_to_fdot6(x1.wrapping_sub(x0));
        let slope = quick_div(dx, dy);
        let abs_slope = slope.wrapping_abs();
        self.x = x0;
        self.dx = slope;
        self.upper_x = x0;
        self.y = y0;
        self.upper_y = y0;
        self.lower_y = y1;
        self.dy = if dx == 0 || slope == 0 {
            MAX_S32
        } else if abs_slope < INVERSE_TABLE_SIZE {
            quick_inverse(abs_slope)
        } else {
            quick_div(dy, dx).wrapping_abs()
        };
        self.edge_type = EdgeType::Line;
        self.curve_count = 0;
        self.winding = winding;
        self.curve_shift = 0;
        true
    }

    fn update_line(
        &mut self,
        mut x0: Fixed,
        mut y0: Fixed,
        mut x1: Fixed,
        mut y1: Fixed,
        slope: Fixed,
    ) -> bool {
        if y0 > y1 {
            std::mem::swap(&mut x0, &mut x1);
            std::mem::swap(&mut y0, &mut y1);
            self.winding = -self.winding;
        }
        let dx = fixed_to_fdot6(x1.wrapping_sub(x0));
        let dy = fixed_to_fdot6(y1.wrapping_sub(y0));
        if dy == 0 {
            return false;
        }
        let abs_slope = fixed_to_fdot6(slope).wrapping_abs();
        self.x = x0;
        self.dx = slope;
        self.upper_x = x0;
        self.y = y0;
        self.upper_y = y0;
        self.lower_y = y1;
        self.dy = if dx == 0 || slope == 0 {
            MAX_S32
        } else if abs_slope < INVERSE_TABLE_SIZE {
            quick_inverse(abs_slope)
        } else {
            quick_div(dy, dx).wrapping_abs()
        };
        true
    }

    fn set_quadratic_without_update(&mut self, pts: &[(f32, f32); 3], shift_aa: i32) -> bool {
        let scale = (1 << (shift_aa + 6)) as f32;
        let mut x0 = (pts[0].0 * scale) as i32;
        let mut y0 = (pts[0].1 * scale) as i32;
        let x1 = (pts[1].0 * scale) as i32;
        let y1 = (pts[1].1 * scale) as i32;
        let mut x2 = (pts[2].0 * scale) as i32;
        let mut y2 = (pts[2].1 * scale) as i32;
        let mut winding = 1i8;
        if y0 > y2 {
            std::mem::swap(&mut x0, &mut x2);
            std::mem::swap(&mut y0, &mut y2);
            winding = -1;
        }
        let top = fdot6_round(y0);
        let bot = fdot6_round(y2);
        if top == bot {
            return false;
        }
        let mut shift = {
            let dx = (left_shift(x1, 1).wrapping_sub(x0).wrapping_sub(x2)) >> 2;
            let dy = (left_shift(y1, 1).wrapping_sub(y0).wrapping_sub(y2)) >> 2;
            diff_to_shift(dx, dy, shift_aa)
        };
        if shift == 0 {
            shift = 1;
        } else if shift > MAX_COEFF_SHIFT {
            shift = MAX_COEFF_SHIFT;
        }
        self.winding = winding;
        self.edge_type = EdgeType::Quad;
        self.curve_count = (1i32 << shift) as i8;
        self.curve_shift = (shift - 1) as u8;

        let a = fdot6_to_fixed_div2(x0.wrapping_sub(x1).wrapping_sub(x1).wrapping_add(x2));
        let b = fdot6_to_fixed(x1.wrapping_sub(x0));
        self.qx = fdot6_to_fixed(x0);
        self.qdx = b.wrapping_add(a >> shift);
        self.qddx = a >> (shift - 1);

        let a = fdot6_to_fixed_div2(y0.wrapping_sub(y1).wrapping_sub(y1).wrapping_add(y2));
        let b = fdot6_to_fixed(y1.wrapping_sub(y0));
        self.qy = fdot6_to_fixed(y0);
        self.qdy = b.wrapping_add(a >> shift);
        self.qddy = a >> (shift - 1);

        self.qlast_x = fdot6_to_fixed(x2);
        self.qlast_y = fdot6_to_fixed(y2);
        true
    }

    fn set_quadratic(&mut self, pts: &[(f32, f32); 3]) -> bool {
        if !self.set_quadratic_without_update(pts, DEFAULT_ACCURACY) {
            return false;
        }
        let a = DEFAULT_ACCURACY;
        self.qx >>= a;
        self.qy >>= a;
        self.qdx >>= a;
        self.qdy >>= a;
        self.qddx >>= a;
        self.qddy >>= a;
        self.qlast_x >>= a;
        self.qlast_y >>= a;
        self.qy = snap_y(self.qy);
        self.qlast_y = snap_y(self.qlast_y);
        self.edge_type = EdgeType::Quad;
        self.snapped_x = self.qx;
        self.snapped_y = self.qy;
        self.update_quadratic()
    }

    fn update_quadratic(&mut self) -> bool {
        let mut success = false;
        let mut count = i32::from(self.curve_count);
        let mut oldx = self.qx;
        let mut oldy = self.qy;
        let mut dx = self.qdx;
        let mut dy = self.qdy;
        let shift = i32::from(self.curve_shift);
        let (mut newx, mut newy, mut new_snapped_x, mut new_snapped_y);
        loop {
            let slope;
            count -= 1;
            if count > 0 {
                newx = oldx.wrapping_add(dx >> shift);
                newy = oldy.wrapping_add(dy >> shift);
                if (dy >> shift).wrapping_abs() >= FIXED1 * 2
                    && (i64::from(dy.wrapping_abs()) << 6) > i64::from(dx.wrapping_abs())
                {
                    let diff_y = fixed_to_fdot6(newy.wrapping_sub(self.snapped_y));
                    slope = if diff_y != 0 {
                        quick_div(fixed_to_fdot6(newx.wrapping_sub(self.snapped_x)), diff_y)
                    } else {
                        MAX_S32
                    };
                    new_snapped_y = self.qlast_y.min(fixed_round_to_fixed(newy));
                    new_snapped_x =
                        newx.wrapping_sub(fixed_mul(slope, newy.wrapping_sub(new_snapped_y)));
                } else {
                    new_snapped_y = self.qlast_y.min(snap_y(newy));
                    new_snapped_x = newx;
                    let diff_y = fixed_to_fdot6(new_snapped_y.wrapping_sub(self.snapped_y));
                    slope = if diff_y != 0 {
                        quick_div(fixed_to_fdot6(newx.wrapping_sub(self.snapped_x)), diff_y)
                    } else {
                        MAX_S32
                    };
                }
                dx = dx.wrapping_add(self.qddx);
                dy = dy.wrapping_add(self.qddy);
            } else {
                newx = self.qlast_x;
                newy = self.qlast_y;
                new_snapped_y = newy;
                new_snapped_x = newx;
                let diff_y = fixed_to_fdot6(newy.wrapping_sub(self.snapped_y));
                slope = if diff_y != 0 {
                    quick_div(fixed_to_fdot6(newx.wrapping_sub(self.snapped_x)), diff_y)
                } else {
                    MAX_S32
                };
            }
            if slope < MAX_S32 {
                success = self.update_line(
                    self.snapped_x,
                    self.snapped_y,
                    new_snapped_x,
                    new_snapped_y,
                    slope,
                );
            }
            oldx = newx;
            oldy = newy;
            if !(count > 0 && !success) {
                break;
            }
        }
        self.qx = newx;
        self.qy = newy;
        self.qdx = dx;
        self.qdy = dy;
        self.snapped_x = new_snapped_x;
        self.snapped_y = new_snapped_y;
        self.curve_count = count as i8;
        let _ = oldx;
        let _ = oldy;
        success
    }

    #[inline]
    fn quad_keep_continuous(&mut self) {
        self.snapped_x = self.x;
        self.snapped_y = self.y;
    }

    fn set_cubic_without_update(&mut self, pts: &[(f32, f32); 4], shift_aa: i32) -> bool {
        let scale = (1 << (shift_aa + 6)) as f32;
        let mut x0 = (pts[0].0 * scale) as i32;
        let mut y0 = (pts[0].1 * scale) as i32;
        let mut x1 = (pts[1].0 * scale) as i32;
        let mut y1 = (pts[1].1 * scale) as i32;
        let mut x2 = (pts[2].0 * scale) as i32;
        let mut y2 = (pts[2].1 * scale) as i32;
        let mut x3 = (pts[3].0 * scale) as i32;
        let mut y3 = (pts[3].1 * scale) as i32;
        let mut winding = 1i8;
        if y0 > y3 {
            std::mem::swap(&mut x0, &mut x3);
            std::mem::swap(&mut x1, &mut x2);
            std::mem::swap(&mut y0, &mut y3);
            std::mem::swap(&mut y1, &mut y2);
            winding = -1;
        }
        let top = fdot6_round(y0);
        let bot = fdot6_round(y3);
        if top == bot {
            return false;
        }
        let mut shift = {
            let dx = cubic_delta_from_line(x0, x1, x2, x3);
            let dy = cubic_delta_from_line(y0, y1, y2, y3);
            diff_to_shift(dx, dy, 2) + 1
        };
        if shift > MAX_COEFF_SHIFT {
            shift = MAX_COEFF_SHIFT;
        }
        let mut up_shift = 6;
        let mut down_shift = shift + up_shift - 10;
        if down_shift < 0 {
            down_shift = 0;
            up_shift = 10 - shift;
        }
        self.winding = winding;
        self.edge_type = EdgeType::Cubic;
        self.curve_count = left_shift(-1, shift) as i8;
        self.curve_shift = shift as u8;
        self.cubic_dshift = down_shift as u8;

        let up = |v: i32| left_shift(v, up_shift);
        let b = up(3i32.wrapping_mul(x1.wrapping_sub(x0)));
        let c = up(3i32.wrapping_mul(x0.wrapping_sub(x1).wrapping_sub(x1).wrapping_add(x2)));
        let d = up(x3
            .wrapping_add(3i32.wrapping_mul(x1.wrapping_sub(x2)))
            .wrapping_sub(x0));
        self.cx = fdot6_to_fixed(x0);
        self.cdx = b.wrapping_add(c >> shift).wrapping_add(d >> (2 * shift));
        self.cddx = (2i32.wrapping_mul(c)).wrapping_add(3i32.wrapping_mul(d) >> (shift - 1));
        self.cdddx = 3i32.wrapping_mul(d) >> (shift - 1);

        let b = up(3i32.wrapping_mul(y1.wrapping_sub(y0)));
        let c = up(3i32.wrapping_mul(y0.wrapping_sub(y1).wrapping_sub(y1).wrapping_add(y2)));
        let d = up(y3
            .wrapping_add(3i32.wrapping_mul(y1.wrapping_sub(y2)))
            .wrapping_sub(y0));
        self.cy = fdot6_to_fixed(y0);
        self.cdy = b.wrapping_add(c >> shift).wrapping_add(d >> (2 * shift));
        self.cddy = (2i32.wrapping_mul(c)).wrapping_add(3i32.wrapping_mul(d) >> (shift - 1));
        self.cdddy = 3i32.wrapping_mul(d) >> (shift - 1);

        self.clast_x = fdot6_to_fixed(x3);
        self.clast_y = fdot6_to_fixed(y3);
        true
    }

    fn set_cubic(&mut self, pts: &[(f32, f32); 4]) -> bool {
        if !self.set_cubic_without_update(pts, DEFAULT_ACCURACY) {
            return false;
        }
        let a = DEFAULT_ACCURACY;
        self.cx >>= a;
        self.cy >>= a;
        self.cdx >>= a;
        self.cdy >>= a;
        self.cddx >>= a;
        self.cddy >>= a;
        self.cdddx >>= a;
        self.cdddy >>= a;
        self.clast_x >>= a;
        self.clast_y >>= a;
        self.cy = snap_y(self.cy);
        self.c_snapped_y = self.cy;
        self.clast_y = snap_y(self.clast_y);
        self.edge_type = EdgeType::Cubic;
        self.update_cubic()
    }

    fn update_cubic(&mut self) -> bool {
        let mut success;
        let mut count = i32::from(self.curve_count);
        let mut oldx = self.cx;
        let mut oldy = self.cy;
        let ddshift = i32::from(self.curve_shift);
        let dshift = i32::from(self.cubic_dshift);
        let (mut newx, mut newy);
        loop {
            count += 1;
            if count < 0 {
                newx = oldx.wrapping_add(self.cdx >> dshift);
                self.cdx = self.cdx.wrapping_add(self.cddx >> ddshift);
                self.cddx = self.cddx.wrapping_add(self.cdddx);
                newy = oldy.wrapping_add(self.cdy >> dshift);
                self.cdy = self.cdy.wrapping_add(self.cddy >> ddshift);
                self.cddy = self.cddy.wrapping_add(self.cdddy);
            } else {
                newx = self.clast_x;
                newy = self.clast_y;
            }
            if newy < oldy {
                newy = oldy;
            }
            let mut new_snapped_y = snap_y(newy);
            if self.clast_y < new_snapped_y {
                new_snapped_y = self.clast_y;
                count = 0;
            }
            let slope = if fixed_to_fdot6(new_snapped_y.wrapping_sub(self.c_snapped_y)) == 0 {
                MAX_S32
            } else {
                fdot6_div(
                    fixed_to_fdot6(newx.wrapping_sub(oldx)),
                    fixed_to_fdot6(new_snapped_y.wrapping_sub(self.c_snapped_y)),
                )
            };
            success = self.update_line(oldx, self.c_snapped_y, newx, new_snapped_y, slope);
            oldx = newx;
            oldy = newy;
            self.c_snapped_y = new_snapped_y;
            if !(count < 0 && !success) {
                break;
            }
        }
        self.cx = newx;
        self.cy = newy;
        self.curve_count = count as i8;
        success
    }

    #[inline]
    fn cubic_keep_continuous(&mut self) {
        self.cx = self.x;
        self.c_snapped_y = self.y;
    }
}

const MAX_COEFF_SHIFT: i32 = 6;

#[inline]
fn cheap_distance(dx: FDot6, dy: FDot6) -> FDot6 {
    let mut dx = dx.wrapping_abs();
    let dy = dy.wrapping_abs();
    if dx > dy {
        dx += dy >> 1;
    } else {
        dx = dy + (dx >> 1);
    }
    dx
}

#[inline]
fn diff_to_shift(dx: FDot6, dy: FDot6, shift_aa: i32) -> i32 {
    let mut dist = cheap_distance(dx, dy);
    dist = (dist + (1 << (2 + shift_aa))) >> (3 + shift_aa);
    (32 - (dist as u32).leading_zeros() as i32) >> 1
}

#[inline]
fn fdot6_to_fixed_div2(value: FDot6) -> Fixed {
    left_shift(value, 16 - 6 - 1)
}

fn cubic_delta_from_line(a: FDot6, b: FDot6, c: FDot6, d: FDot6) -> FDot6 {
    let one_third = (a
        .wrapping_mul(8)
        .wrapping_sub(b.wrapping_mul(15))
        .wrapping_add(6i32.wrapping_mul(c))
        .wrapping_add(d))
    .wrapping_mul(19)
        >> 9;
    let two_third = (a
        .wrapping_add(6i32.wrapping_mul(b))
        .wrapping_sub(c.wrapping_mul(15))
        .wrapping_add(d.wrapping_mul(8)))
    .wrapping_mul(19)
        >> 9;
    one_third.wrapping_abs().max(two_third.wrapping_abs())
}

// ---------------------------------------------------------------------------
// Geometry (SkGeometry.cpp): monotonic chops in Y.

fn valid_unit_divide(mut numer: f32, mut denom: f32) -> Option<f32> {
    if numer < 0.0 {
        numer = -numer;
        denom = -denom;
    }
    if denom == 0.0 || numer == 0.0 || numer >= denom {
        return None;
    }
    let r = numer / denom;
    if r.is_nan() || r == 0.0 {
        return None;
    }
    Some(r)
}

fn is_not_monotonic(a: f32, b: f32, c: f32) -> bool {
    let ab = a - b;
    let mut bc = b - c;
    if ab < 0.0 {
        bc = -bc;
    }
    ab == 0.0 || bc < 0.0
}

type Pt = (f32, f32);

#[inline]
fn mix(a: Pt, b: Pt, t: f32) -> Pt {
    ((b.0 - a.0) * t + a.0, (b.1 - a.1) * t + a.1)
}

#[inline]
fn interp(a: Pt, b: Pt, t: f32) -> Pt {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// `SkChopQuadAtYExtrema`: 1 or 2 monotonic quads.
fn chop_quad_at_y_extrema(src: [Pt; 3]) -> ([Pt; 5], usize) {
    let (a, mut b, c) = (src[0].1, src[1].1, src[2].1);
    if is_not_monotonic(a, b, c) {
        if let Some(t) = valid_unit_divide(a - b, a - b - b + c) {
            let p01 = interp(src[0], src[1], t);
            let p12 = interp(src[1], src[2], t);
            let mut dst = [src[0], p01, interp(p01, p12, t), p12, src[2]];
            // flatten_double_quad_extrema: y of 1, 3 = y of 2.
            dst[1].1 = dst[2].1;
            dst[3].1 = dst[2].1;
            return (dst, 2);
        }
        b = if (a - b).abs() < (b - c).abs() { a } else { c };
    }
    (
        [
            (src[0].0, a),
            (src[1].0, b),
            (src[2].0, c),
            (0.0, 0.0),
            (0.0, 0.0),
        ],
        1,
    )
}

fn find_unit_quad_roots(a: f32, b: f32, c: f32) -> Vec<f32> {
    if a == 0.0 {
        return valid_unit_divide(-c, b).into_iter().collect();
    }
    let dr = f64::from(b) * f64::from(b) - 4.0 * f64::from(a) * f64::from(c);
    if dr < 0.0 {
        return Vec::new();
    }
    let r = dr.sqrt() as f32;
    if !r.is_finite() {
        return Vec::new();
    }
    let q = if b < 0.0 {
        -(b - r) / 2.0
    } else {
        -(b + r) / 2.0
    };
    let mut roots: Vec<f32> = Vec::with_capacity(2);
    roots.extend(valid_unit_divide(q, a));
    roots.extend(valid_unit_divide(c, q));
    if roots.len() == 2 {
        if roots[0] > roots[1] {
            roots.swap(0, 1);
        } else if roots[0] == roots[1] {
            roots.pop();
        }
    }
    roots
}

fn chop_cubic_at(src: &[Pt; 4], t: f32) -> [Pt; 7] {
    if t == 1.0 {
        return [src[0], src[1], src[2], src[3], src[3], src[3], src[3]];
    }
    let ab = mix(src[0], src[1], t);
    let bc = mix(src[1], src[2], t);
    let cd = mix(src[2], src[3], t);
    let abc = mix(ab, bc, t);
    let bcd = mix(bc, cd, t);
    let abcd = mix(abc, bcd, t);
    [src[0], ab, abc, abcd, bcd, cd, src[3]]
}

fn chop_cubic_at2(src: &[Pt; 4], t0: f32, t1: f32) -> [Pt; 10] {
    if t1 == 1.0 {
        let a = chop_cubic_at(src, t0);
        return [
            a[0], a[1], a[2], a[3], a[4], a[5], a[6], src[3], src[3], src[3],
        ];
    }
    let lane = |t: f32| {
        let ab = mix(src[0], src[1], t);
        let bc = mix(src[1], src[2], t);
        let cd = mix(src[2], src[3], t);
        let abc = mix(ab, bc, t);
        let bcd = mix(bc, cd, t);
        let abcd = mix(abc, bcd, t);
        (ab, bc, cd, abc, bcd, abcd)
    };
    let lo = lane(t0);
    let hi = lane(t1);
    // middle = mix(abc, bcd, shuffle(T)): the low lanes take t1, the high t0.
    let middle_lo = mix(lo.3, lo.4, t1);
    let middle_hi = mix(hi.3, hi.4, t0);
    [
        src[0], lo.0, lo.3, lo.5, middle_lo, middle_hi, hi.5, hi.4, hi.2, src[3],
    ]
}

/// `SkChopCubicAtYExtrema`: 1 to 3 monotonic cubics (points 0..=3, 3..=6,
/// 6..=9).
fn chop_cubic_at_y_extrema(src: [Pt; 4]) -> ([Pt; 10], usize) {
    let (a, b, c, d) = (src[0].1, src[1].1, src[2].1, src[3].1);
    let ca = d - a + 3.0 * (b - c);
    let cb = 2.0 * (a - b - b + c);
    let cc = b - a;
    let t = find_unit_quad_roots(ca, cb, cc);
    let mut dst = [(0.0f32, 0.0f32); 10];
    match t.len() {
        0 => {
            dst[..4].copy_from_slice(&src);
        }
        1 => {
            let c7 = chop_cubic_at(&src, t[0]);
            dst[..7].copy_from_slice(&c7);
        }
        _ => {
            dst = chop_cubic_at2(&src, t[0], t[1]);
        }
    }
    let roots = t.len();
    if roots > 0 {
        // flatten_double_cubic_extrema(&dst[0].fY): y of 2, 4 = y of 3.
        dst[2].1 = dst[3].1;
        dst[4].1 = dst[3].1;
        if roots == 2 {
            dst[5].1 = dst[6].1;
            dst[7].1 = dst[6].1;
        }
    }
    (dst, roots + 1)
}

// ---------------------------------------------------------------------------
// Edge building (SkEdgeBuilder, analytic, no clip).

/// A path segment in device space.
#[derive(Clone, Copy, Debug)]
pub enum Seg {
    Line(Pt, Pt),
    Quad(Pt, Pt, Pt),
    Cubic(Pt, Pt, Pt, Pt),
}

/// `SkPathEdgeIter` over `pm_skia_path::Path`: each contour's segments and its
/// closing line (even when not closed explicitly, and even when empty).
pub fn path_segments(path: &pm_skia_path::Path) -> Vec<Seg> {
    let mut out = Vec::new();
    let mut last = (0.0f32, 0.0f32);
    let mut start = (0.0f32, 0.0f32);
    let mut needs_close = false;
    let p = |q: pm_skia_path::Point| (q.x, q.y);
    for seg in path.segments() {
        match seg {
            pm_skia_path::PathSegment::MoveTo(m) => {
                if needs_close {
                    out.push(Seg::Line(last, start));
                    needs_close = false;
                }
                start = p(m);
                last = start;
            }
            pm_skia_path::PathSegment::LineTo(a) => {
                out.push(Seg::Line(last, p(a)));
                last = p(a);
                needs_close = true;
            }
            pm_skia_path::PathSegment::QuadTo(c, a) => {
                out.push(Seg::Quad(last, p(c), p(a)));
                last = p(a);
                needs_close = true;
            }
            pm_skia_path::PathSegment::ConicTo(c, a, w) => {
                // `SkEdgeBuilder`: a conic becomes quads in device space
                // (`SkAutoConicToQuads`, tolerance ¼ px).
                let from = pm_skia_path::Point::from_xy(last.0, last.1);
                let quads = pm_skia_path::path_geometry::skia_conic_to_quads(from, c, a, w, 0.25);
                for q in quads[1..].chunks_exact(2) {
                    out.push(Seg::Quad(last, p(q[0]), p(q[1])));
                    last = p(q[1]);
                }
                needs_close = true;
            }
            pm_skia_path::PathSegment::CubicTo(c0, c1, a) => {
                out.push(Seg::Cubic(last, p(c0), p(c1), p(a)));
                last = p(a);
                needs_close = true;
            }
            pm_skia_path::PathSegment::Close => {
                if needs_close {
                    out.push(Seg::Line(last, start));
                    needs_close = false;
                }
            }
        }
    }
    if needs_close {
        out.push(Seg::Line(last, start));
    }
    out
}

enum Combine {
    No,
    Partial,
    Total,
}

fn combine_vertical(edge: &Edge, last: &mut Edge) -> Combine {
    let approx = |a: Fixed, b: Fixed| a.wrapping_sub(b).wrapping_abs() < 0x100;
    if last.edge_type != EdgeType::Line || last.dx != 0 || edge.x != last.x {
        return Combine::No;
    }
    if edge.winding == last.winding {
        if edge.lower_y == last.upper_y {
            last.upper_y = edge.upper_y;
            last.y = last.upper_y;
            return Combine::Partial;
        }
        if approx(edge.upper_y, last.lower_y) {
            last.lower_y = edge.lower_y;
            return Combine::Partial;
        }
        return Combine::No;
    }
    if approx(edge.upper_y, last.upper_y) {
        if approx(edge.lower_y, last.lower_y) {
            return Combine::Total;
        }
        if edge.lower_y < last.lower_y {
            last.upper_y = edge.lower_y;
            last.y = last.upper_y;
            return Combine::Partial;
        }
        last.upper_y = last.lower_y;
        last.y = last.upper_y;
        last.lower_y = edge.lower_y;
        last.winding = edge.winding;
        return Combine::Partial;
    }
    if approx(edge.lower_y, last.lower_y) {
        if edge.upper_y > last.upper_y {
            last.lower_y = edge.upper_y;
            return Combine::Partial;
        }
        last.lower_y = last.upper_y;
        last.upper_y = edge.upper_y;
        last.y = last.upper_y;
        last.winding = edge.winding;
        return Combine::Partial;
    }
    Combine::No
}

fn build_edges(segs: &[Seg]) -> Vec<Edge> {
    let mut list: Vec<Edge> = Vec::new();
    let add_line = |list: &mut Vec<Edge>, a: Pt, b: Pt| {
        let mut e = Edge::default();
        if e.set_line(a, b) {
            let vertical = e.dx == 0 && e.edge_type == EdgeType::Line;
            let combine = if vertical && !list.is_empty() {
                let last = list.last_mut().unwrap();
                combine_vertical(&e, last)
            } else {
                Combine::No
            };
            match combine {
                Combine::Total => {
                    list.pop();
                }
                Combine::Partial => {}
                Combine::No => list.push(e),
            }
        }
    };
    for s in segs {
        match *s {
            Seg::Line(a, b) => add_line(&mut list, a, b),
            Seg::Quad(a, b, c) => {
                let (dst, n) = chop_quad_at_y_extrema([a, b, c]);
                for i in 0..n {
                    let mut e = Edge::default();
                    if e.set_quadratic(&[dst[i * 2], dst[i * 2 + 1], dst[i * 2 + 2]]) {
                        list.push(e);
                    }
                }
            }
            Seg::Cubic(a, b, c, d) => {
                let (dst, n) = chop_cubic_at_y_extrema([a, b, c, d]);
                for i in 0..n {
                    let mut e = Edge::default();
                    if e.set_cubic(&[dst[i * 3], dst[i * 3 + 1], dst[i * 3 + 2], dst[i * 3 + 3]]) {
                        list.push(e);
                    }
                }
            }
        }
    }
    list
}

// ---------------------------------------------------------------------------
// Sorting (SkTSort.h), with `compare_edges`.

fn edge_less(a: &Edge, b: &Edge) -> bool {
    if a.upper_y != b.upper_y {
        return a.upper_y < b.upper_y;
    }
    if a.x != b.x {
        return a.x < b.x;
    }
    a.dx < b.dx
}

fn insertion_sort<T: Copy>(v: &mut [T], less: &impl Fn(&T, &T) -> bool) {
    for next in 1..v.len() {
        if !less(&v[next], &v[next - 1]) {
            continue;
        }
        let insert = v[next];
        let mut hole = next;
        loop {
            v[hole] = v[hole - 1];
            hole -= 1;
            if !(0 < hole && less(&insert, &v[hole - 1])) {
                break;
            }
        }
        v[hole] = insert;
    }
}

fn heap_sift_up<T: Copy>(
    a: &mut [T],
    mut root: usize,
    bottom: usize,
    less: &impl Fn(&T, &T) -> bool,
) {
    let x = a[root - 1];
    let start = root;
    let mut j = root << 1;
    while j <= bottom {
        if j < bottom && less(&a[j - 1], &a[j]) {
            j += 1;
        }
        a[root - 1] = a[j - 1];
        root = j;
        j = root << 1;
    }
    j = root >> 1;
    while j >= start {
        if less(&a[j - 1], &x) {
            a[root - 1] = a[j - 1];
            root = j;
            j = root >> 1;
        } else {
            break;
        }
    }
    a[root - 1] = x;
}

fn heap_sift_down<T: Copy>(
    a: &mut [T],
    mut root: usize,
    bottom: usize,
    less: &impl Fn(&T, &T) -> bool,
) {
    let x = a[root - 1];
    let mut child = root << 1;
    while child <= bottom {
        if child < bottom && less(&a[child - 1], &a[child]) {
            child += 1;
        }
        if less(&x, &a[child - 1]) {
            a[root - 1] = a[child - 1];
            root = child;
            child = root << 1;
        } else {
            break;
        }
    }
    a[root - 1] = x;
}

fn heap_sort<T: Copy>(a: &mut [T], less: &impl Fn(&T, &T) -> bool) {
    let count = a.len();
    let mut i = count >> 1;
    while i > 0 {
        heap_sift_down(a, i, count, less);
        i -= 1;
    }
    let mut i = count - 1;
    while i > 0 {
        a.swap(0, i);
        heap_sift_up(a, 1, i, less);
        i -= 1;
    }
}

fn partition<T: Copy>(v: &mut [T], pivot: usize, less: &impl Fn(&T, &T) -> bool) -> usize {
    let right = v.len() - 1;
    let pivot_value = v[pivot];
    v.swap(pivot, right);
    let mut new_pivot = 0;
    let mut left = 0;
    while left < right {
        if less(&v[left], &pivot_value) {
            v.swap(left, new_pivot);
            new_pivot += 1;
        }
        left += 1;
    }
    v.swap(new_pivot, right);
    new_pivot
}

fn intro_sort<T: Copy>(mut depth: i32, mut v: &mut [T], less: &impl Fn(&T, &T) -> bool) {
    loop {
        if v.len() <= 32 {
            insertion_sort(v, less);
            return;
        }
        if depth == 0 {
            heap_sort(v, less);
            return;
        }
        depth -= 1;
        let middle = (v.len() - 1) >> 1;
        let p = partition(v, middle, less);
        let (left, rest) = v.split_at_mut(p);
        intro_sort(depth, left, less);
        v = &mut rest[1..];
    }
}

/// `SkTQSort`.
fn sk_qsort<T: Copy>(v: &mut [T], less: impl Fn(&T, &T) -> bool) {
    let n = v.len();
    if n <= 1 {
        return;
    }
    // 2 * SkNextLog2(n - 1), SkNextLog2(x) = 32 - clz(x - 1).
    let next_log2 = 32 - ((n as u32 - 2).leading_zeros() as i32);
    intro_sort(2 * next_log2, v, &less);
}

// ---------------------------------------------------------------------------
// Blitters.

/// `SK_ColorBLACK` premultiplied: the paint for which `SkBlitter::Choose`
/// selects `SkARGB32_Black_Blitter` (`paint.getColor() == SK_ColorBLACK`;
/// only unpremultiplied (0, 0, 0, 255) premultiplies to this).
const SK_PM_BLACK: [u32; 4] = [0, 0, 0, 255];

/// The A8 destination with Skia's source-over A8 blitter (paint alpha 255).
struct Device {
    w: i32,
    h: i32,
    data: Vec<u8>,
    // Premultiplied N32 paint, or an A8 white coverage destination.
    color: Option<[u32; 4]>,
}

#[inline]
fn div255(prod: u32) -> u8 {
    (((prod + 128) * 257) >> 16) as u8
}

#[inline]
fn srcover(src: u8, dst: u8) -> u8 {
    src.wrapping_add(div255((255 - u32::from(src)) * u32::from(dst)))
}

impl Device {
    #[inline]
    fn px(&mut self, x: i32, y: i32) -> Option<&mut u8> {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return None;
        }
        Some(&mut self.data[(y * self.w + x) as usize])
    }
    /// One pixel of `blitAntiH` with a run alpha.
    #[inline]
    fn aa(&mut self, x: i32, y: i32, aa: u8) {
        if aa == 0 {
            return;
        }
        if let Some(color) = self.color {
            if let Some(i) = self.rgba_index(x, y) {
                let src = color.map(|v| v * (u32::from(aa) + 1) >> 8);
                for c in 0..4 {
                    self.data[i + c] = (src[c]
                        + (u32::from(self.data[i + c]) * (256 - src[3]) >> 8))
                        .min(255) as u8;
                }
            }
            return;
        }
        if let Some(d) = self.px(x, y) {
            // canFoldAA: src = div255(255 × aa) = aa; srcover.
            let src = if aa == 0xFF {
                0xFF
            } else {
                div255(255 * u32::from(aa))
            };
            *d = srcover(src, *d);
        }
    }
    fn rgba_index(&self, x: i32, y: i32) -> Option<usize> {
        (x >= 0 && y >= 0 && x < self.w && y < self.h).then_some(((y * self.w + x) * 4) as usize)
    }
    // `blitAntiH2` of the N32 blitter `SkBlitter::Choose` picks for a solid
    // paint (SkBlitter.cpp:750-756): black, opaque or translucent. The
    // opaque and translucent forms combine multiply and add before
    // quantizing (SkFastFourByteInterp, SkBlendARGB32) rather than
    // quantizing the source before SkPMSrcOver; black adds the raw alpha.
    fn aa_two(&mut self, x: i32, y: i32, aa: u8) {
        let Some(color) = self.color else {
            self.aa(x, y, aa);
            return;
        };
        if let Some(i) = self.rgba_index(x, y) {
            if color == SK_PM_BLACK {
                // SkARGB32_Black_Blitter::blitAntiH2
                // (SkBlitter_ARGB32.cpp:1724-1730):
                // (a << SK_A32_SHIFT) + SkAlphaMulQ(dst, 256 - a).
                let scale = 256 - u32::from(aa);
                for c in 0..4 {
                    self.data[i + c] = ((u32::from(self.data[i + c]) * scale) >> 8) as u8;
                }
                self.data[i + 3] += aa;
                return;
            }
            let scale = if color[3] == 255 {
                u32::from(aa) + (u32::from(aa) >> 7)
            } else {
                u32::from(aa) + 1
            };
            let keep = if color[3] == 255 {
                256 - scale
            } else {
                let prod = 0xffff - color[3] * scale;
                (prod + (prod >> 8)) >> 8
            };
            for c in 0..4 {
                self.data[i + c] =
                    ((color[c] * scale + u32::from(self.data[i + c]) * keep) >> 8) as u8;
            }
        }
    }
    fn mask_pixel(&mut self, x: i32, y: i32, m: u8) {
        if self.color.is_some() {
            self.aa(x, y, m);
        } else if let Some(d) = self.px(x, y) {
            let one = srcover(255, *d);
            *d = div255((255 - u32::from(m)) * u32::from(*d) + u32::from(m) * u32::from(one));
        }
    }
    fn blit_v(&mut self, x: i32, y: i32, height: i32, aa: u8) {
        for i in 0..height {
            self.aa(x, y + i, aa);
        }
    }
    fn blit_h(&mut self, x: i32, y: i32, width: i32) {
        for i in 0..width {
            self.aa(x + i, y, 0xFF);
        }
    }
}

#[inline]
fn safely_add_alpha(alpha: &mut u8, delta: u8) {
    *alpha = (u32::from(*alpha) + u32::from(delta)).min(0xFF) as u8;
}

/// `MaskAdditiveBlitter` (masks up to 32 px wide and 1024 bytes) or
/// `SafeRLEAdditiveBlitter`, over the device.
enum Additive {
    Mask {
        /// The mask image starts one byte into `buf` and its rows are
        /// `width` bytes apart, so a write one pixel left or right of a row
        /// lands in the neighboring row, as in Skia's storage (one byte of
        /// slack at each end of the whole mask).
        left: i32,
        top: i32,
        width: i32,
        height: i32,
        buf: Vec<u8>,
    },
    Rle {
        left: i32,
        width: i32,
        top: i32,
        curr_y: i32,
        row: Vec<u8>,
        dirty: bool,
    },
}

impl Additive {
    fn mask_row(&mut self, y: i32) -> Option<MaskRow> {
        match self {
            Additive::Mask {
                left, top, width, ..
            } => Some(MaskRow {
                offset: 1 + ((y - *top) * *width) as isize - *left as isize,
            }),
            Additive::Rle { .. } => None,
        }
    }

    fn mask_add(&mut self, row: MaskRow, x: i32, delta: u8) {
        if let Additive::Mask { buf, .. } = self {
            let i = row.offset + x as isize;
            if i >= 0 && (i as usize) < buf.len() {
                safely_add_alpha(&mut buf[i as usize], delta);
            }
        }
    }

    fn mask_set(&mut self, row: MaskRow, x: i32, value: u8) {
        if let Additive::Mask { buf, .. } = self {
            let i = row.offset + x as isize;
            if i >= 0 && (i as usize) < buf.len() {
                buf[i as usize] = value;
            }
        }
    }

    fn flush(&mut self, dev: &mut Device) {
        if let Additive::Rle {
            left,
            top,
            curr_y,
            row,
            dirty,
            ..
        } = self
            && *curr_y >= *top
        {
            if *dirty {
                for (i, a) in row.iter_mut().enumerate() {
                    // snapAlpha: blitting 0 and 255 is faster.
                    let v = if *a > 247 {
                        0xFF
                    } else if *a < 8 {
                        0
                    } else {
                        *a
                    };
                    dev.aa(*left + i as i32, *curr_y, v);
                    *a = 0;
                }
            }
            *dirty = false;
            *curr_y = *top - 1;
        }
    }

    fn check_y(&mut self, y: i32, dev: &mut Device) {
        if let Additive::Rle { curr_y, .. } = self
            && *curr_y != y
        {
            self.flush(dev);
            if let Additive::Rle { curr_y, .. } = self {
                *curr_y = y;
            }
        }
    }

    /// `blitAntiH(x, y, antialias[], len)`.
    fn blit_anti_h_array(&mut self, x: i32, y: i32, alphas: &[u8], dev: &mut Device) {
        self.check_y(y, dev);
        if let Additive::Rle {
            left,
            width,
            row,
            dirty,
            ..
        } = self
        {
            let mut x = x - *left;
            let mut alphas = alphas;
            let mut len = alphas.len() as i32;
            if x < 0 {
                len += x;
                alphas = &alphas[(-x) as usize..];
                x = 0;
            }
            len = len.min(*width - x);
            for i in 0..len.max(0) {
                safely_add_alpha(&mut row[(x + i) as usize], alphas[i as usize]);
            }
            *dirty = true;
        }
    }

    /// `blitAntiH(x, y, alpha)`.
    fn blit_anti_h1(&mut self, x: i32, y: i32, alpha: u8, dev: &mut Device) {
        self.check_y(y, dev);
        if let Additive::Rle {
            left,
            width,
            row,
            dirty,
            ..
        } = self
        {
            let x = x - *left;
            if x >= 0 && x < *width {
                safely_add_alpha(&mut row[x as usize], alpha);
                *dirty = true;
            }
        }
    }

    /// `blitAntiH(x, y, width, alpha)`.
    fn blit_anti_hn(&mut self, x: i32, y: i32, n: i32, alpha: u8, dev: &mut Device) {
        self.check_y(y, dev);
        if let Additive::Rle {
            left,
            width,
            row,
            dirty,
            ..
        } = self
        {
            let x = x - *left;
            if x >= 0 && x + n <= *width {
                for i in 0..n {
                    safely_add_alpha(&mut row[(x + i) as usize], alpha);
                }
                *dirty = true;
            }
        }
    }

    fn finish(&mut self, dev: &mut Device) {
        match self {
            Additive::Mask {
                left,
                top,
                width,
                height,
                buf,
            } => {
                // blitMask: dst = lerp(dst, srcover(255, dst), m).
                for yy in 0..*height {
                    for xx in 0..*width {
                        let m = buf[(1 + yy * *width + xx) as usize];
                        dev.mask_pixel(*left + xx, *top + yy, m);
                    }
                }
            }
            Additive::Rle { .. } => self.flush(dev),
        }
    }
}

#[derive(Clone, Copy)]
struct MaskRow {
    offset: isize,
}

// ---------------------------------------------------------------------------
// Alpha computation (SkScan_AAAPath.cpp).

#[inline]
fn trapezoid_to_alpha(l1: Fixed, l2: Fixed) -> u8 {
    let area = (l1.wrapping_add(l2)) / 2;
    (area >> 8) as u8
}

#[inline]
fn partial_triangle_to_alpha(a: Fixed, b: Fixed) -> u8 {
    let area = (a >> 11).wrapping_mul(a >> 11).wrapping_mul(b >> 11);
    ((area >> 8) & 0xFF) as u8
}

/// `get_partial_alpha(SkAlpha, SkFixed)`.
#[inline]
fn partial_alpha_fixed(alpha: u8, partial_height: Fixed) -> u8 {
    fixed_round_to_int(i32::from(alpha).wrapping_mul(partial_height)) as u8
}

/// `get_partial_alpha(SkAlpha, SkAlpha)`.
#[inline]
fn partial_alpha(alpha: u8, full_alpha: u8) -> u8 {
    ((u32::from(alpha) * u32::from(full_alpha)) >> 8) as u8
}

#[inline]
fn fixed_to_alpha(f: Fixed) -> u8 {
    partial_alpha_fixed(0xFF, f)
}

fn approximate_intersection(mut l1: Fixed, mut r1: Fixed, mut l2: Fixed, mut r2: Fixed) -> Fixed {
    if l1 > r1 {
        std::mem::swap(&mut l1, &mut r1);
    }
    if l2 > r2 {
        std::mem::swap(&mut l2, &mut r2);
    }
    (l1.max(l2).wrapping_add(r1.min(r2))) / 2
}

fn compute_alpha_above_line(alphas: &mut [u8], l: Fixed, r: Fixed, dy: Fixed, full_alpha: u8) {
    let rr = fixed_ceil_to_int(r);
    if rr == 0 {
        return;
    } else if rr == 1 {
        let v = (((rr << 17).wrapping_sub(l).wrapping_sub(r)) >> 9) as u8;
        alphas[0] = partial_alpha(v, full_alpha);
    } else {
        let first = FIXED1 - l;
        let last = r - ((rr - 1) << 16);
        let first_h = fixed_mul(first, dy);
        alphas[0] = (fixed_mul(first, first_h) >> 9) as u8;
        let mut alpha16 = sat_add(first_h, dy >> 1);
        for a in alphas.iter_mut().take((rr - 1) as usize).skip(1) {
            *a = (alpha16 >> 8) as u8;
            alpha16 = sat_add(alpha16, dy);
        }
        alphas[(rr - 1) as usize] = full_alpha.wrapping_sub(partial_triangle_to_alpha(last, dy));
    }
}

fn compute_alpha_below_line(alphas: &mut [u8], l: Fixed, r: Fixed, dy: Fixed, full_alpha: u8) {
    let rr = fixed_ceil_to_int(r);
    if rr == 0 {
        return;
    } else if rr == 1 {
        alphas[0] = partial_alpha(trapezoid_to_alpha(l, r), full_alpha);
    } else {
        let first = FIXED1 - l;
        let last = r - ((rr - 1) << 16);
        let last_h = fixed_mul(last, dy);
        alphas[(rr - 1) as usize] = (fixed_mul(last, last_h) >> 9) as u8;
        let mut alpha16 = sat_add(last_h, dy >> 1);
        let mut i = rr - 2;
        while i > 0 {
            alphas[i as usize] = ((alpha16 >> 8) & 0xFF) as u8;
            alpha16 = sat_add(alpha16, dy);
            i -= 1;
        }
        alphas[0] = full_alpha.wrapping_sub(partial_triangle_to_alpha(first, dy));
    }
}

struct Ctx<'a> {
    dev: &'a mut Device,
    add: &'a mut Additive,
}

impl Ctx<'_> {
    fn single_alpha(
        &mut self,
        y: i32,
        x: i32,
        alpha: u8,
        full_alpha: u8,
        mask: Option<MaskRow>,
        no_real: bool,
    ) {
        if let Some(row) = mask {
            if full_alpha == 0xFF && !no_real {
                self.add.mask_set(row, x, alpha);
            } else {
                self.add.mask_add(row, x, partial_alpha(alpha, full_alpha));
            }
        } else if full_alpha == 0xFF && !no_real {
            self.dev.blit_v(x, y, 1, alpha);
        } else {
            self.add
                .blit_anti_h1(x, y, partial_alpha(alpha, full_alpha), self.dev);
        }
    }

    fn two_alphas(
        &mut self,
        y: i32,
        x: i32,
        a1: u8,
        a2: u8,
        full_alpha: u8,
        mask: Option<MaskRow>,
        no_real: bool,
    ) {
        if let Some(row) = mask {
            self.add.mask_add(row, x, a1);
            self.add.mask_add(row, x + 1, a2);
        } else if full_alpha == 0xFF && !no_real {
            // blitAntiH2: two one-pixel runs.
            self.dev.aa_two(x, y, a1);
            self.dev.aa_two(x + 1, y, a2);
        } else {
            self.add.blit_anti_h1(x, y, a1, self.dev);
            self.add.blit_anti_h1(x + 1, y, a2, self.dev);
        }
    }

    fn full_alpha(
        &mut self,
        y: i32,
        x: i32,
        len: i32,
        full_alpha: u8,
        mask: Option<MaskRow>,
        no_real: bool,
    ) {
        if let Some(row) = mask {
            for i in 0..len {
                self.add.mask_add(row, x + i, full_alpha);
            }
        } else if full_alpha == 0xFF && !no_real {
            self.dev.blit_h(x, y, len);
        } else {
            self.add.blit_anti_hn(x, y, len, full_alpha, self.dev);
        }
    }

    fn aaa_trapezoid_row(
        &mut self,
        y: i32,
        ul: Fixed,
        ur: Fixed,
        ll: Fixed,
        lr: Fixed,
        l_dy: Fixed,
        r_dy: Fixed,
        full_alpha: u8,
        mask: Option<MaskRow>,
        no_real: bool,
    ) {
        let l = fixed_floor_to_int(ul);
        let r = fixed_ceil_to_int(lr);
        let len = r - l;
        if len == 1 {
            let alpha = trapezoid_to_alpha(ur.wrapping_sub(ul), lr.wrapping_sub(ll));
            self.single_alpha(y, l, alpha, full_alpha, mask, no_real);
            return;
        }
        if len <= 0 {
            return;
        }
        let n = len as usize;
        let mut alphas = vec![full_alpha; n + 1];
        let mut temp = vec![0u8; n + 2];

        let u_l = fixed_floor_to_int(ul);
        let l_l = fixed_ceil_to_int(ll);
        if u_l + 2 == l_l {
            let first = int_to_fixed(u_l) + FIXED1 - ul;
            let second = ll - ul - first;
            let a1 = full_alpha.wrapping_sub(partial_triangle_to_alpha(first, l_dy));
            let a2 = partial_triangle_to_alpha(second, l_dy);
            alphas[0] = if alphas[0] > a1 { alphas[0] - a1 } else { 0 };
            alphas[1] = if alphas[1] > a2 { alphas[1] - a2 } else { 0 };
        } else {
            let start = (u_l - l) as usize;
            compute_alpha_below_line(
                &mut temp[start..],
                ul - int_to_fixed(u_l),
                ll - int_to_fixed(u_l),
                l_dy,
                full_alpha,
            );
            for i in u_l..l_l {
                let k = (i - l) as usize;
                alphas[k] = if alphas[k] > temp[k] {
                    alphas[k] - temp[k]
                } else {
                    0
                };
            }
        }

        let u_r = fixed_floor_to_int(ur);
        let l_r = fixed_ceil_to_int(lr);
        if u_r + 2 == l_r {
            let first = int_to_fixed(u_r) + FIXED1 - ur;
            let second = lr - ur - first;
            let a1 = partial_triangle_to_alpha(first, r_dy);
            let a2 = full_alpha.wrapping_sub(partial_triangle_to_alpha(second, r_dy));
            alphas[n - 2] = if alphas[n - 2] > a1 {
                alphas[n - 2] - a1
            } else {
                0
            };
            alphas[n - 1] = if alphas[n - 1] > a2 {
                alphas[n - 1] - a2
            } else {
                0
            };
        } else {
            let start = (u_r - l) as usize;
            compute_alpha_above_line(
                &mut temp[start..],
                ur - int_to_fixed(u_r),
                lr - int_to_fixed(u_r),
                r_dy,
                full_alpha,
            );
            for i in u_r..l_r {
                let k = (i - l) as usize;
                alphas[k] = if alphas[k] > temp[k] {
                    alphas[k] - temp[k]
                } else {
                    0
                };
            }
        }

        if let Some(row) = mask {
            for (i, &a) in alphas[..n].iter().enumerate() {
                self.add.mask_add(row, l + i as i32, a);
            }
        } else if full_alpha == 0xFF && !no_real {
            for (i, &a) in alphas[..n].iter().enumerate() {
                self.dev.aa(l + i as i32, y, a);
            }
        } else {
            self.add.blit_anti_h_array(l, y, &alphas[..n], self.dev);
        }
    }

    fn trapezoid_row(
        &mut self,
        y: i32,
        mut ul: Fixed,
        mut ur: Fixed,
        mut ll: Fixed,
        mut lr: Fixed,
        l_dy: Fixed,
        r_dy: Fixed,
        full_alpha: u8,
        mask: Option<MaskRow>,
        no_real: bool,
    ) {
        if ul > ur {
            return;
        }
        if ll > lr {
            let v = approximate_intersection(ul, ll, ur, lr);
            ll = v;
            lr = v;
        }
        if ul == ur && ll == lr {
            return;
        }
        if ul > ll {
            std::mem::swap(&mut ul, &mut ll);
        }
        if ur > lr {
            std::mem::swap(&mut ur, &mut lr);
        }
        let join_left = fixed_ceil_to_fixed(ll);
        let join_rite = fixed_floor_to_fixed(ur);
        if join_left <= join_rite {
            if ul < join_left {
                let len = fixed_ceil_to_int(join_left - ul);
                if len == 1 {
                    let alpha = trapezoid_to_alpha(join_left - ul, join_left - ll);
                    self.single_alpha(y, ul >> 16, alpha, full_alpha, mask, no_real);
                } else if len == 2 {
                    let first = join_left - FIXED1 - ul;
                    let second = ll - ul - first;
                    let a1 = partial_triangle_to_alpha(first, l_dy);
                    let a2 = full_alpha.wrapping_sub(partial_triangle_to_alpha(second, l_dy));
                    self.two_alphas(y, ul >> 16, a1, a2, full_alpha, mask, no_real);
                } else {
                    self.aaa_trapezoid_row(
                        y, ul, join_left, ll, join_left, l_dy, MAX_S32, full_alpha, mask, no_real,
                    );
                }
            }
            if join_left < join_rite {
                self.full_alpha(
                    y,
                    fixed_floor_to_int(join_left),
                    fixed_floor_to_int(join_rite - join_left),
                    full_alpha,
                    mask,
                    no_real,
                );
            }
            if lr > join_rite {
                let len = fixed_ceil_to_int(lr - join_rite);
                if len == 1 {
                    let alpha = trapezoid_to_alpha(ur - join_rite, lr - join_rite);
                    self.single_alpha(y, join_rite >> 16, alpha, full_alpha, mask, no_real);
                } else if len == 2 {
                    let first = join_rite + FIXED1 - ur;
                    let second = lr - ur - first;
                    let a1 = full_alpha.wrapping_sub(partial_triangle_to_alpha(first, r_dy));
                    let a2 = partial_triangle_to_alpha(second, r_dy);
                    self.two_alphas(y, join_rite >> 16, a1, a2, full_alpha, mask, no_real);
                } else {
                    self.aaa_trapezoid_row(
                        y, join_rite, ur, join_rite, lr, MAX_S32, r_dy, full_alpha, mask, no_real,
                    );
                }
            }
        } else {
            self.aaa_trapezoid_row(y, ul, ur, ll, lr, l_dy, r_dy, full_alpha, mask, no_real);
        }
    }
}

// ---------------------------------------------------------------------------
// The edge walk (aaa_walk_edges) over an arena: 0 is the head, 1 the tail.

const HEAD: usize = 0;
const TAIL: usize = 1;

struct Edges(Vec<Edge>);

impl Edges {
    #[inline]
    fn remove(&mut self, e: usize) {
        let (p, n) = (self.0[e].prev, self.0[e].next);
        self.0[p].next = n;
        self.0[n].prev = p;
    }
    #[inline]
    fn insert_after(&mut self, e: usize, after: usize) {
        let an = self.0[after].next;
        self.0[e].prev = after;
        self.0[e].next = an;
        self.0[an].prev = e;
        self.0[after].next = e;
    }
    fn backward_insert_based_on_x(&mut self, e: usize) {
        let x = self.0[e].x;
        let mut prev = self.0[e].prev;
        while self.0[prev].prev != NONE && self.0[prev].x > x {
            prev = self.0[prev].prev;
        }
        if self.0[prev].next != e {
            self.remove(e);
            self.insert_after(e, prev);
        }
    }
    fn backward_insert_start(&self, mut prev: usize, x: Fixed) -> usize {
        while self.0[prev].prev != NONE && self.0[prev].x > x {
            prev = self.0[prev].prev;
        }
        prev
    }
}

#[inline]
fn update_next_next_y(y: Fixed, next_y: Fixed, next_next_y: &mut Fixed) {
    if y > next_y && y < *next_next_y {
        *next_next_y = y;
    }
}

fn check_intersection(edges: &Edges, e: usize, next_y: Fixed, next_next_y: &mut Fixed) {
    let p = edges.0[e].prev;
    if edges.0[p].prev != NONE
        && edges.0[p].x.wrapping_add(edges.0[p].dx) > edges.0[e].x.wrapping_add(edges.0[e].dx)
    {
        *next_next_y = next_y + (FIXED1 >> DEFAULT_ACCURACY);
    }
}

fn check_intersection_fwd(edges: &Edges, e: usize, next_y: Fixed, next_next_y: &mut Fixed) {
    let n = edges.0[e].next;
    if edges.0[n].next != NONE
        && edges.0[e].x.wrapping_add(edges.0[e].dx) > edges.0[n].x.wrapping_add(edges.0[n].dx)
    {
        *next_next_y = next_y + (FIXED1 >> DEFAULT_ACCURACY);
    }
}

fn insert_new_edges(edges: &mut Edges, mut new_edge: usize, y: Fixed, next_next_y: &mut Fixed) {
    if edges.0[new_edge].upper_y > y {
        update_next_next_y(edges.0[new_edge].upper_y, y, next_next_y);
        return;
    }
    let prev = edges.0[new_edge].prev;
    if edges.0[prev].x <= edges.0[new_edge].x {
        while edges.0[new_edge].upper_y <= y {
            check_intersection(edges, new_edge, y, next_next_y);
            update_next_next_y(edges.0[new_edge].lower_y, y, next_next_y);
            new_edge = edges.0[new_edge].next;
        }
        update_next_next_y(edges.0[new_edge].upper_y, y, next_next_y);
        return;
    }
    let mut start = edges.backward_insert_start(prev, edges.0[new_edge].x);
    loop {
        let next = edges.0[new_edge].next;
        let mut skip = false;
        loop {
            if edges.0[start].next == new_edge {
                skip = true;
                break;
            }
            let after = edges.0[start].next;
            if edges.0[after].x >= edges.0[new_edge].x {
                break;
            }
            start = after;
        }
        if !skip {
            edges.remove(new_edge);
            edges.insert_after(new_edge, start);
        }
        check_intersection(edges, new_edge, y, next_next_y);
        check_intersection_fwd(edges, new_edge, y, next_next_y);
        update_next_next_y(edges.0[new_edge].lower_y, y, next_next_y);
        start = new_edge;
        new_edge = next;
        if edges.0[new_edge].upper_y > y {
            break;
        }
    }
    update_next_next_y(edges.0[new_edge].upper_y, y, next_next_y);
}

fn edges_too_close(edges: &Edges, prev: usize, next: usize, lower_y: Fixed) -> bool {
    const SLACK: Fixed = FIXED1;
    next != NONE
        && prev != NONE
        && edges.0[next].upper_y < lower_y
        && edges.0[prev].x.wrapping_add(SLACK)
            >= edges.0[next]
                .x
                .wrapping_sub(edges.0[next].dx.wrapping_abs())
}

fn edges_too_close_rite(prev_rite: i32, ul: Fixed, ll: Fixed) -> bool {
    prev_rite > fixed_floor_to_int(ul) || prev_rite > fixed_floor_to_int(ll)
}

fn aaa_walk_edges(
    edges: &mut Edges,
    ctx: &mut Ctx<'_>,
    start_y: i32,
    stop_y: i32,
    left_clip: Fixed,
    right_clip: Fixed,
    using_mask: bool,
    skip_intersect: bool,
) {
    edges.0[HEAD].x = left_clip;
    edges.0[HEAD].upper_x = left_clip;
    edges.0[TAIL].x = right_clip;
    edges.0[TAIL].upper_x = right_clip;
    let first = edges.0[HEAD].next;
    let mut y = edges.0[first].upper_y.max(int_to_fixed(start_y));
    let mut next_next_y = MAX_S32;
    {
        let mut e = edges.0[HEAD].next;
        while edges.0[e].upper_y <= y {
            edges.0[e].go_y(y);
            update_next_next_y(edges.0[e].lower_y, y, &mut next_next_y);
            e = edges.0[e].next;
        }
        update_next_next_y(edges.0[e].upper_y, y, &mut next_next_y);
    }
    // Winding fill, not inverse.
    let winding_mask = -1i32;
    loop {
        let mut w = 0i32;
        let mut in_interval = false;
        let mut prev_x = edges.0[HEAD].x;
        let mut next_y = next_next_y.min(fixed_ceil_to_fixed(y.wrapping_add(1)));
        let mut curr = edges.0[HEAD].next;
        let mut left_e = HEAD;
        let mut left = left_clip;
        let mut left_dy: Fixed = 0;
        let mut prev_rite = fixed_floor_to_int(left_clip);
        next_next_y = MAX_S32;

        let mut y_shift = 0;
        if (next_y - y) & (FIXED1 >> 2) != 0 {
            y_shift = 2;
            next_y = y + (FIXED1 >> 2);
        } else if (next_y - y) & (FIXED1 >> 1) != 0 {
            y_shift = 1;
        }
        let full_alpha = fixed_to_alpha(next_y - y);
        let mask = if using_mask {
            ctx.add.mask_row(fixed_floor_to_int(y))
        } else {
            None
        };
        let no_real_blitter = false;

        while edges.0[curr].upper_y <= y {
            w += i32::from(edges.0[curr].winding);
            let prev_in_interval = in_interval;
            in_interval = (w & winding_mask) != 0;
            let is_left = in_interval && !prev_in_interval;
            let is_rite = !in_interval && prev_in_interval;
            if is_rite {
                let mut rite = edges.0[curr].x;
                edges.0[curr].go_y_shift(next_y, y_shift);
                let next_left = left_clip.max(edges.0[left_e].x);
                rite = right_clip.min(rite);
                let next_rite = right_clip.min(edges.0[curr].x);
                let curr_next = edges.0[curr].next;
                let no_real = no_real_blitter
                    || (full_alpha == 0xFF
                        && (edges_too_close_rite(prev_rite, left, edges.0[left_e].x)
                            || edges_too_close(edges, curr, curr_next, next_y)));
                let curr_dy = edges.0[curr].dy;
                ctx.trapezoid_row(
                    y >> 16,
                    left,
                    rite,
                    next_left,
                    next_rite,
                    left_dy,
                    curr_dy,
                    full_alpha,
                    mask,
                    no_real,
                );
                prev_rite = fixed_ceil_to_int(rite.max(edges.0[curr].x));
            } else {
                if is_left {
                    left = edges.0[curr].x.max(left_clip);
                    left_dy = edges.0[curr].dy;
                    left_e = curr;
                }
                edges.0[curr].go_y_shift(next_y, y_shift);
            }

            let next = edges.0[curr].next;
            while edges.0[curr].lower_y <= next_y {
                let e = &mut edges.0[curr];
                if e.curve_count < 0 {
                    e.cubic_keep_continuous();
                    if !e.update_cubic() {
                        break;
                    }
                } else if e.curve_count > 0 {
                    e.quad_keep_continuous();
                    if !e.update_quadratic() {
                        break;
                    }
                } else {
                    break;
                }
            }

            if edges.0[curr].lower_y <= next_y {
                edges.remove(curr);
            } else {
                update_next_next_y(edges.0[curr].lower_y, next_y, &mut next_next_y);
                let new_x = edges.0[curr].x;
                if new_x < prev_x {
                    edges.backward_insert_based_on_x(curr);
                } else {
                    prev_x = new_x;
                }
                if !skip_intersect {
                    check_intersection(edges, curr, next_y, &mut next_next_y);
                }
            }
            curr = next;
        }

        if in_interval {
            let lp = edges.0[left_e].prev;
            let no_real = no_real_blitter
                || (full_alpha == 0xFF && edges_too_close(edges, lp, left_e, next_y));
            let next_left = left_clip.max(edges.0[left_e].x);
            ctx.trapezoid_row(
                y >> 16,
                left,
                right_clip,
                next_left,
                right_clip,
                left_dy,
                0,
                full_alpha,
                mask,
                no_real,
            );
        }

        y = next_y;
        if y >= int_to_fixed(stop_y) {
            break;
        }
        insert_new_edges(edges, curr, y, &mut next_next_y);
    }
}

/// Fills `path` (device pixels of a `width` × `height` A8 mask, winding
/// fill) as `SkDraw::drawPath` with an anti-aliased paint into a cleared A8
/// pixmap: Skia's analytic coverage, exactly.
pub fn fill_path_a8(path: &pm_skia_path::Path, width: u32, height: u32) -> Vec<u8> {
    fill_path(
        path,
        width,
        height,
        Device {
            w: width as i32,
            h: height as i32,
            data: vec![0; (width * height) as usize],
            color: None,
        },
    )
}

/// Skia's solid N32 span blitter over an existing premultiplied RGBA target.
/// Keep the analytic fill's span order: collapsing it to one A8 mask loses
/// the two-pixel blitter's combined multiply/add and intermediate rounding.
pub fn fill_path_solid(
    path: &pm_skia_path::Path,
    width: u32,
    height: u32,
    color: [f32; 4],
    pixels: &mut Vec<u8>,
) {
    assert_eq!(pixels.len(), (width * height * 4) as usize);
    let a = (color[3] * 255.).round().clamp(0., 255.) as u32;
    let paint = std::array::from_fn(|c| {
        if c == 3 {
            a
        } else {
            u32::from(div255((color[c] * 255.).round().clamp(0., 255.) as u32 * a))
        }
    });
    *pixels = fill_path(
        path,
        width,
        height,
        Device {
            w: width as i32,
            h: height as i32,
            data: std::mem::take(pixels),
            color: Some(paint),
        },
    );
}

fn fill_path(path: &pm_skia_path::Path, width: u32, height: u32, mut dev: Device) -> Vec<u8> {
    let (w, h) = (width as i32, height as i32);
    let b = path.bounds();
    // safeRoundOut of the path bounds; the clip is the mask.
    let ir = (
        b.left().floor() as i32,
        b.top().floor() as i32,
        b.right().ceil() as i32,
        b.bottom().ceil() as i32,
    );
    let clip = (0, 0, w, h);
    // Intersect with the clip.
    let ix = (
        ir.0.max(clip.0),
        ir.1.max(clip.1),
        ir.2.min(clip.2),
        ir.3.min(clip.3),
    );
    if ix.0 >= ix.2 || ix.1 >= ix.3 || ir.0 >= ir.2 || ir.1 >= ir.3 {
        return dev.data;
    }
    let contained = ir.0 >= clip.0 && ir.1 >= clip.1 && ir.2 <= clip.2 && ir.3 <= clip.3;
    let segs = path_segments(path);
    let mut list = build_edges(&segs);
    // Paths not contained in the mask would need Skia's edge clipper; glyph
    // masks contain their path by construction.
    if !contained || list.is_empty() {
        return dev.data;
    }
    let ir_w = ir.2 - ir.0;
    let ir_h = ir.3 - ir.1;
    let can_mask = ir_w <= 32 && i64::from((ir_w + 3) & !3) * i64::from(ir_h) <= 1024;
    if can_mask && convex::fat_rect(path, &mut dev) {
        return dev.data;
    }
    let mut add = if can_mask {
        Additive::Mask {
            left: ir.0,
            top: ir.1,
            width: ir_w,
            height: ir_h,
            buf: vec![0; (ir_w * ir_h + 2) as usize],
        }
    } else {
        Additive::Rle {
            left: ix.0,
            width: ix.2 - ix.0,
            top: ix.1,
            curr_y: ix.1 - 1,
            row: vec![0; (ix.2 - ix.0) as usize],
            dirty: false,
        }
    };
    sk_qsort(&mut list, edge_less);
    let n = list.len();
    let mut arena = Vec::with_capacity(n + 2);
    let mut head = Edge::default();
    head.prev = NONE;
    head.next = 2;
    head.upper_y = MIN_S32;
    head.lower_y = MIN_S32;
    head.x = MIN_S32;
    head.dx = 0;
    head.dy = MAX_S32;
    head.upper_x = MIN_S32;
    let mut tail = Edge::default();
    tail.prev = n + 1;
    tail.next = NONE;
    tail.upper_y = MAX_S32;
    tail.lower_y = MAX_S32;
    tail.x = MAX_S32;
    tail.dx = 0;
    tail.dy = MAX_S32;
    tail.upper_x = MAX_S32;
    arena.push(head);
    arena.push(tail);
    for (i, mut e) in list.into_iter().enumerate() {
        e.prev = if i == 0 { HEAD } else { i + 1 };
        e.next = if i + 1 == n { TAIL } else { i + 3 };
        arena.push(e);
    }
    let mut edges = Edges(arena);
    let (start_y, stop_y) = (ir.1, ir.3);
    let mut left_bound = int_to_fixed(clip.0);
    let mut right_bound = int_to_fixed(clip.2);
    if can_mask {
        left_bound = left_bound.max(int_to_fixed(ir.0));
        right_bound = right_bound.min(int_to_fixed(ir.2));
    }
    // Skip intersections when the path has many points.
    let points = path.points().len();
    let skip_intersect = points > ((stop_y - start_y) * 2) as usize;
    {
        let mut ctx = Ctx {
            dev: &mut dev,
            add: &mut add,
        };
        if path.is_convex() && n >= 2 {
            convex::walk(
                &mut edges,
                &mut ctx,
                stop_y,
                left_bound,
                right_bound,
                can_mask,
            );
        } else {
            aaa_walk_edges(
                &mut edges,
                &mut ctx,
                start_y,
                stop_y,
                left_bound,
                right_bound,
                can_mask,
                skip_intersect,
            );
        }
    }
    add.finish(&mut dev);
    dev.data
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_path(x0: f32, y0: f32, x1: f32, y1: f32) -> pm_skia_path::Path {
        let mut b = pm_skia_path::PathBuilder::new();
        b.move_to(x0, y0);
        b.line_to(x1, y0);
        b.line_to(x1, y1);
        b.line_to(x0, y1);
        b.close();
        b.finish().unwrap()
    }

    #[test]
    fn whole_and_half_pixels() {
        let m = fill_path_a8(&rect_path(1.0, 1.0, 3.0, 3.0), 4, 4);
        assert_eq!(
            m,
            vec![0, 0, 0, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 0, 0, 0]
        );
        let m = fill_path_a8(&rect_path(0.5, 0.0, 2.0, 1.0), 2, 1);
        assert_eq!(m[1], 255);
        assert!((i32::from(m[0]) - 128).abs() <= 1, "{}", m[0]);
    }

    #[test]
    fn a_triangle_covers_half_its_box() {
        let mut b = pm_skia_path::PathBuilder::new();
        b.move_to(0.0, 0.0);
        b.line_to(40.0, 0.0);
        b.line_to(0.0, 40.0);
        b.close();
        let p = b.finish().unwrap();
        let m = fill_path_a8(&p, 40, 40);
        let sum: u32 = m.iter().map(|&v| u32::from(v)).sum();
        let area = sum as f64 / 255.0;
        assert!((area - 800.0).abs() < 2.0, "{area}");
    }

    #[test]
    fn sorting_matches_a_total_order() {
        let mut v: Vec<i32> = (0..200).map(|i| (i * 7919) % 211).collect();
        sk_qsort(&mut v, |a, b| a < b);
        assert!(v.windows(2).all(|w| w[0] <= w[1]));
    }
}
