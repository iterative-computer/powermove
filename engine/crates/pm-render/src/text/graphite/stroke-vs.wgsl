// Generated from Skia chrome/m152 747cf654ed6f864f4402e7d6eb54a6ede60141e5.
// Skia, Copyright Google LLC, BSD-3-Clause. See pm-skia-path/LICENSE.
diagnostic(off, derivative_uniformity);
struct VSIn {
@builtin(vertex_index) sk_VertexID: u32,
@location(0) p01: vec4<f32>,
@location(1) p23: vec4<f32>,
@location(2) prevPoint: vec2<f32>,
@location(3) stroke: vec2<f32>,
@location(4) depth: f32,
@location(5) @interpolate(flat, either) ssboIndex: u32,
@location(6) curveType: f32,
};
struct VSOut {
@builtin(position) sk_Position: vec4<f32>,
};
/* unsupported */ var<private> sk_PointSize: f32;
struct Params {
u: array<vec4<f32>, 12>,
};
@group(0) @binding(0) var<uniform> _uniform0 : Params;
fn inverse_2x2f(m: mat2x2<f32>) -> mat2x2<f32> {
return mat2x2<f32>(m[1].y, -m[0].y, -m[1].x, m[0].x) * (1/determinant(m));
}
fn m_Qff2f2f2f2f22(a: vec2<f32>, b: vec2<f32>, c: vec2<f32>, d: vec2<f32>, e: mat2x2<f32>) -> f32 {
{
let f: vec2<f32> = e * (fma(vec2<f32>(-2.0), b, c) + a);
let g: vec2<f32> = e * (fma(vec2<f32>(-2.0), c, d) + b);
return max(dot(f, f), dot(g, g));
}
}
fn p_Qff2f2f2f(_skParam0: vec2<f32>, _skParam1: vec2<f32>, _skParam2: vec2<f32>, d: f32) -> f32 {
var a = _skParam0;
var b = _skParam1;
var c = _skParam2;
{
let e: vec2<f32> = (min(min(a, b), c) + max(max(a, b), c)) * 0.5;
a = a - e;
b = b - e;
c = c - e;
let f: f32 = sqrt(max(max(dot(a, a), dot(b, b)), dot(c, c)));
let g: vec2<f32> = fma(vec2<f32>(-2.0 * d), b, a) + c;
let h: f32 = abs(fma(-2.0, d, 2.0));
let i: f32 = max(0.0, fma(f, 4.0, -1.0));
let j: f32 = length(g) * 4.0 + i * h;
let k: f32 = 4.0 * min(d, 1.0);
return j / k;
}
}
fn s_Qf2f2f2(c: vec2<f32>, d: vec2<f32>) -> vec2<f32> {
{
let e: vec2<f32> = c - d;
if all(e == vec2<f32>(0.0)) {
return vec2<f32>(0.0);
} else {
{
let f: f32 = 1.0 / max(abs(e.x), abs(e.y));
return normalize(f * e);
}
}
}
return vec2<f32>();
}
fn w_Qf2f2f2f(c: vec2<f32>, d: vec2<f32>, e: f32) -> vec2<f32> {
{
return fma(d - c, vec2<f32>(e), c);
}
}
fn _skslMain(_stageIn: VSIn, _stageOut: ptr<function, VSOut>) {
{
var edgeID: f32 = f32(i32(_stageIn.sk_VertexID) >> 1);
if (i32(_stageIn.sk_VertexID) & 1) != 0 {
{
edgeID = -edgeID;
}
}
var _0_tessellate_stroked_curve: vec4<f32>;
let _skTemp1 = _uniform0.u[1].xy;
let _skTemp2 = _uniform0.u[1].zw;
let _1_c: mat2x2<f32> = mat2x2<f32>(_skTemp1[0], _skTemp1[1], _skTemp2[0], _skTemp2[1]);
var _2_h: vec2<f32> = _stageIn.prevPoint;
var _3_k: vec2<f32> = _stageIn.p01.xy;
var _4_l: vec2<f32> = _stageIn.p01.zw;
var _5_m: vec2<f32> = _stageIn.p23.xy;
var _6_n: vec2<f32> = _stageIn.p23.zw;
var _7_o: f32 = -1.0;
if _stageIn.curveType != 0.0 {
{
_7_o = _6_n.x;
_6_n = _5_m;
}
}
var _8_p: f32;
if _7_o < 0.0 {
if all(_3_k == _4_l) && all(_5_m == _6_n) {
_8_p = 1.0;
} else {
{
let _9_f: f32 = m_Qff2f2f2f2f22(_3_k, _4_l, _5_m, _6_n, _1_c);
_8_p = max(ceil(sqrt(3.0 * sqrt(_9_f))), 1.0);
}
}
} else {
if all(_3_k == _4_l) || all(_4_l == _5_m) {
_8_p = 1.0;
} else {
{
let _10_e: f32 = p_Qff2f2f2f(_1_c * _3_k, _1_c * _4_l, _1_c * _5_m, _7_o);
_8_p = max(ceil(sqrt(_10_e)), 1.0);
}
}
}
var _11_q: f32 = _stageIn.stroke.x;
var _12_r: f32 = _stageIn.stroke.y;
let _13_s: bool = (_stageIn.stroke.x == 0.0);
var _14_t: f32;
if _13_s {
{
_14_t = 0.477464825;
_11_q = 0.5;
}
} else {
{
_14_t = 0.5 / acos(max(1.0 - 0.25 / (_uniform0.u[2].z * _stageIn.stroke.x), -1.0));
}
}
if _13_s {
{
_3_k = _1_c * _3_k;
_4_l = _1_c * _4_l;
_5_m = _1_c * _5_m;
_6_n = _1_c * _6_n;
_2_h = _1_c * _2_h;
}
}
var _skTemp3: vec2<f32>;
if all(_3_k == _4_l) {
_skTemp3 = select(_5_m, _6_n, vec2<bool>(all(_4_l == _5_m)));
} else {
_skTemp3 = _4_l;
}
var _15_u: vec2<f32> = s_Qf2f2f2(_skTemp3, _3_k);
var _skTemp4: vec2<f32>;
if all(_6_n == _5_m) {
_skTemp4 = select(_4_l, _3_k, vec2<bool>(all(_5_m == _4_l)));
} else {
_skTemp4 = _5_m;
}
var _16_v: vec2<f32> = s_Qf2f2f2(_6_n, _skTemp4);
var _17_x: vec2<f32> = s_Qf2f2f2(_3_k, _2_h);
if all(_15_u == vec2<f32>(0.0)) {
{
_12_r = 0.0;
if _7_o < 0.0 {
{
_15_u = vec2<f32>(1.0, 0.0);
_16_v = vec2<f32>(-1.0, 0.0);
}
} else {
{
_7_o = -1.0;
_15_u = _17_x;
_16_v = _17_x;
if all(_17_x == vec2<f32>(0.0)) {
{
_6_n = _3_k + _11_q * vec2<f32>(1.0, 0.0);
_5_m = _6_n;
_4_l = _3_k - _11_q * vec2<f32>(1.0, 0.0);
_3_k = _4_l;
_16_v = vec2<f32>(1.0, 0.0);
_15_u = _16_v;
_17_x = _15_u;
}
} else {
_6_n = _3_k + _11_q * _17_x;
_5_m = _6_n;
}
}
}
}
}
var _18_y: f32;
if _12_r >= 0.0 {
_18_y = sign(_12_r) + 3.0;
} else {
{
let _19_z: f32 = acos(clamp(dot(_17_x, _15_u), -1.0, 1.0));
let _20_A: f32 = max(ceil(_19_z * _14_t), 1.0);
_18_y = _20_A + 2.0;
_18_y = min(_18_y, 16381.0);
}
}
let _skTemp5 = _5_m - _3_k;
let _skTemp6 = _6_n - _4_l;
var _21_z: f32 = determinant(mat2x2<f32>(_skTemp5[0], _skTemp5[1], _skTemp6[0], _skTemp6[1]));
var _22_A: f32 = abs(edgeID) - _18_y;
if _22_A < 0.0 {
{
_16_v = _15_u;
if any(_2_h != _3_k) {
_15_u = _17_x;
}
_21_z = determinant(mat2x2<f32>(_15_u[0], _15_u[1], _16_v[0], _16_v[1]));
}
}
let _23_B: f32 = clamp(dot(_15_u, _16_v), -1.0, 1.0);
var _24_C: f32 = acos(_23_B);
if _21_z < 0.0 {
_24_C = -_24_C;
}
var _25_D: f32;
var _26_E: f32 = sign(edgeID);
if _22_A < 0.0 {
{
_25_D = _18_y - 2.0;
_8_p = 1.0;
_4_l = _3_k;
_5_m = _4_l;
_6_n = _5_m;
_22_A = _22_A + (_25_D + 1.0);
if _22_A < 0.0 {
_22_A = 0.0;
} else {
{
let _27_G: bool = (abs(_21_z) * inverseSqrt(dot(_15_u, _15_u) * dot(_16_v, _16_v))) < 0.01;
if (!_27_G) || (dot(_15_u, _16_v) < 0.0) {
var _skTemp7: f32;
if _21_z < 0.0 {
_skTemp7 = min(_26_E, 0.0);
} else {
_skTemp7 = max(_26_E, 0.0);
}
_26_E = _skTemp7;
}
}
}
}
} else {
{
let _28_F: f32 = (16383.0 - _18_y) - 1.0;
_25_D = max(ceil(abs(_24_C) * _14_t), 1.0);
_25_D = min(_25_D, _28_F);
_8_p = min(_8_p, (_28_F - _25_D) + 1.0);
}
}
let _29_F: f32 = _24_C / _25_D;
let _30_G: f32 = (_8_p + _25_D) - 1.0;
let _31_H: bool = _22_A >= _30_G;
if _22_A > _30_G {
_26_E = 0.0;
}
if (abs(edgeID) == 2.0) && (_12_r > 0.0) {
{
let _32_c: f32 = fma(_23_B, 0.5, 0.5);
var _skTemp8: f32;
if ((_32_c * _12_r) * _12_r) >= 1.0 {
_skTemp8 = inverseSqrt(_32_c);
} else {
_skTemp8 = sqrt(_32_c);
}
_26_E = _26_E * _skTemp8;
}
}
var _33_I: vec2<f32>;
var _34_J: vec2<f32>;
if (_22_A != 0.0) && (!_31_H) {
{
var _35_K: vec2<f32>;
var _36_L: vec2<f32>;
var _37_M: vec2<f32> = _4_l - _3_k;
let _38_N: vec2<f32> = _6_n - _3_k;
if _7_o >= 0.0 {
{
_37_M = _37_M * _7_o;
_36_L = 0.5 * _38_N - _37_M;
_35_K = (_7_o - 1.0) * _38_N;
_4_l = _4_l * _7_o;
}
} else {
{
let _39_O: vec2<f32> = _5_m - _4_l;
_36_L = _39_O - _37_M;
_35_K = fma(vec2<f32>(-3.0), _39_O, _38_N);
}
}
let _40_O: vec2<f32> = _36_L * (_8_p * 2.0);
let _41_P: vec2<f32> = _37_M * (_8_p * _8_p);
var _42_Q: f32 = 0.0;
let _43_R: f32 = min(_8_p - 1.0, _22_A);
let _44_S: f32 = -abs(_29_F);
let _45_U: f32 = (1.0 + _22_A) * abs(_29_F);
{
var _46_V: f32 = 32.0;
for (; _46_V >= 1.0; _46_V = _46_V * 0.5) {
{
let _47_W: f32 = _42_Q + _46_V;
if _47_W <= _43_R {
{
var _48_X: vec2<f32> = fma(vec2<f32>(_47_W), _35_K, _40_O);
_48_X = fma(vec2<f32>(_47_W), _48_X, _41_P);
let _49_Y: f32 = dot(normalize(_48_X), _15_u);
var _50_Z: f32 = fma(_47_W, _44_S, _45_U);
_50_Z = min(_50_Z, 3.14159274);
if _49_Y >= cos(_50_Z) {
_42_Q = _47_W;
}
}
}
}
}
}
let _51_V: f32 = _42_Q / _8_p;
let _52_W: f32 = _22_A - _42_Q;
var _53_X: f32 = acos(clamp(_15_u.x, -1.0, 1.0));
_53_X = select(-_53_X, _53_X, _15_u.y >= 0.0);
let _54_Y: f32 = fma(_52_W, _29_F, _53_X);
_33_I = vec2<f32>(cos(_54_Y), sin(_54_Y));
let _55_Z: vec2<f32> = vec2<f32>(-_33_I.y, _33_I.x);
let _56_aa: f32 = dot(_55_Z, _35_K);
let _57_ac: f32 = dot(_55_Z, _36_L);
let _58_ad: f32 = dot(_55_Z, _37_M);
let _59_ae: f32 = max(_57_ac * _57_ac - _56_aa * _58_ad, 0.0);
var _60_af: f32 = sqrt(_59_ae);
if _57_ac > 0.0 {
_60_af = -_60_af;
}
_60_af = _60_af - _57_ac;
let _61_ag: f32 = (-0.5 * _60_af) * _56_aa;
var _skTemp9: vec2<f32>;
if abs(fma(_60_af, _60_af, _61_ag)) < abs(fma(_56_aa, _58_ad, _61_ag)) {
_skTemp9 = vec2<f32>(_60_af, _56_aa);
} else {
_skTemp9 = vec2<f32>(_58_ad, _60_af);
}
let _62_ah: vec2<f32> = _skTemp9;
var _skTemp10: f32;
if (_52_W != 0.0) && (_62_ah.y != 0.0) {
_skTemp10 = saturate(_62_ah.x / _62_ah.y);
} else {
_skTemp10 = 0.0;
}
let _63_ai: f32 = _skTemp10;
let _64_aj: f32 = max(_51_V, _63_ai);
let _65_ak: vec2<f32> = w_Qf2f2f2f(_3_k, _4_l, _64_aj);
let _66_al: vec2<f32> = w_Qf2f2f2f(_4_l, _5_m, _64_aj);
let _67_am: vec2<f32> = w_Qf2f2f2f(_5_m, _6_n, _64_aj);
let _68_an: vec2<f32> = w_Qf2f2f2f(_65_ak, _66_al, _64_aj);
let _69_ao: vec2<f32> = w_Qf2f2f2f(_66_al, _67_am, _64_aj);
let _70_ap: vec2<f32> = w_Qf2f2f2f(_68_an, _69_ao, _64_aj);
let _71_aq: f32 = fma(_7_o - 1.0, _64_aj, 1.0);
let _72_ar: f32 = (_7_o + 1.0) - _71_aq;
let _73_as: f32 = fma(_72_ar - _71_aq, _64_aj, _71_aq);
if _64_aj != _63_ai {
var _skTemp11: vec2<f32>;
if _7_o >= 0.0 {
_skTemp11 = s_Qf2f2f2(_66_al * _71_aq, _65_ak * _72_ar);
} else {
_skTemp11 = s_Qf2f2f2(_69_ao, _68_an);
}
_33_I = _skTemp11;
}
var _skTemp12: vec2<f32>;
if _7_o >= 0.0 {
_skTemp12 = _68_an / _73_as;
} else {
_skTemp12 = _70_ap;
}
_34_J = _skTemp12;
}
} else {
{
_33_I = select(_16_v, _15_u, vec2<bool>(_22_A == 0.0));
_34_J = select(_6_n, _3_k, vec2<bool>(_22_A == 0.0));
}
}
let _74_K: vec2<f32> = vec2<f32>(_33_I.y, -_33_I.x);
_34_J = _34_J + _74_K * (_11_q * _26_E);
if _13_s {
_0_tessellate_stroked_curve = vec4<f32>(_34_J + _uniform0.u[2].xy, inverse_2x2f(_1_c) * _34_J);
} else {
_0_tessellate_stroked_curve = vec4<f32>(_1_c * _34_J + _uniform0.u[2].xy, _34_J);
}
let coords: vec4<f32> = _0_tessellate_stroked_curve;
let devPosition: vec4<f32> = vec4<f32>(coords.xy, _stageIn.depth, 1.0);
(*_stageOut).sk_Position = vec4<f32>(_uniform0.u[0].zw * devPosition.xy - sign(_uniform0.u[0].zw) * devPosition.ww, devPosition.zw);
}
}
@vertex fn main(_stageIn: VSIn) -> VSOut {
var _stageOut: VSOut;
_skslMain(_stageIn, &_stageOut);
return _stageOut;
}
