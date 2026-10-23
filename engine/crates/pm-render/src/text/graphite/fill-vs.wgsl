// Generated from Skia chrome/m152 747cf654ed6f864f4402e7d6eb54a6ede60141e5.
// Skia, Copyright Google LLC, BSD-3-Clause. See pm-skia-path/LICENSE.
diagnostic(off, derivative_uniformity);
struct VSIn {
@location(0) resolveLevel_and_idx: vec2<f32>,
@location(1) p01: vec4<f32>,
@location(2) p23: vec4<f32>,
@location(3) fanPoint: vec2<f32>,
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
fn _skslMain(_stageIn: VSIn, _stageOut: ptr<function, VSOut>) {
{
let _skTemp1 = _uniform0.u[1];
let _skTemp2 = _uniform0.u[2];
let _skTemp3 = _uniform0.u[3];
let _skTemp4 = _uniform0.u[4];
let localToDevice: mat4x4<f32> = mat4x4<f32>(_skTemp1[0], _skTemp1[1], _skTemp1[2], _skTemp1[3], _skTemp2[0], _skTemp2[1], _skTemp2[2], _skTemp2[3], _skTemp3[0], _skTemp3[1], _skTemp3[2], _skTemp3[3], _skTemp4[0], _skTemp4[1], _skTemp4[2], _skTemp4[3]);
var localCoord: vec2<f32>;
if _stageIn.resolveLevel_and_idx.x < 0.0 {
{
localCoord = _stageIn.fanPoint;
}
} else {
{
let _skTemp5 = localToDevice[0].xy;
let _skTemp6 = localToDevice[1].xy;
let vectorXform: mat2x2<f32> = mat2x2<f32>(_skTemp5[0], _skTemp5[1], _skTemp6[0], _skTemp6[1]);
var _0_b: f32 = _stageIn.resolveLevel_and_idx.x;
var _1_c: f32 = _stageIn.resolveLevel_and_idx.y;
var _2_g: vec2<f32>;
if _stageIn.curveType == 2.0 {
var _skTemp7: vec2<f32>;
if _0_b != 0.0 {
_skTemp7 = _stageIn.p01.zw;
} else {
_skTemp7 = select(_stageIn.p01.xy, _stageIn.p23.xy, vec2<bool>(_1_c != 0.0));
}
_2_g = _skTemp7;
} else {
{
let _3_h: vec2<f32> = _stageIn.p01.xy;
var _4_i: vec2<f32> = _stageIn.p01.zw;
let _5_j: vec2<f32> = _stageIn.p23.xy;
var _6_k: vec2<f32> = _stageIn.p23.zw;
var _7_l: f32 = -1.0;
var _8_m: f32;
if _stageIn.curveType != 0.0 {
{
_7_l = _6_k.x;
let _9_e: f32 = p_Qff2f2f2f(vectorXform * _3_h, vectorXform * _4_i, vectorXform * _5_j, _7_l);
_8_m = ceil(log2(max(_9_e, 1.0)) * 0.5);
_4_i = _4_i * _7_l;
_6_k = _5_j;
}
} else {
{
let _10_f: f32 = m_Qff2f2f2f2f22(_3_h, _4_i, _5_j, _6_k, vectorXform);
_8_m = ceil(log2(max(9.0 * _10_f, 1.0)) * 0.25);
}
}
if _0_b > _8_m {
{
_1_c = floor(ldexp(_1_c, i32(_8_m - _0_b)));
_0_b = _8_m;
}
}
let _11_n: f32 = floor(0.5 + ldexp(_1_c, i32(5.0 - _0_b)));
if (0.0 < _11_n) && (_11_n < 32.0) {
{
let _12_o: f32 = _11_n * 0.03125;
let _13_p: vec2<f32> = mix(_3_h, _4_i, vec2<f32>(_12_o));
let _14_q: vec2<f32> = mix(_4_i, _5_j, vec2<f32>(_12_o));
let _15_r: vec2<f32> = mix(_5_j, _6_k, vec2<f32>(_12_o));
let _16_s: vec2<f32> = mix(_13_p, _14_q, vec2<f32>(_12_o));
let _17_t: vec2<f32> = mix(_14_q, _15_r, vec2<f32>(_12_o));
let _18_x: vec2<f32> = mix(_16_s, _17_t, vec2<f32>(_12_o));
let _19_y: f32 = mix(1.0, _7_l, _12_o);
let _20_z: f32 = (_7_l + 1.0) - _19_y;
let _21_A: f32 = mix(_19_y, _20_z, _12_o);
var _skTemp8: vec2<f32>;
if _7_l < 0.0 {
_skTemp8 = _18_x;
} else {
_skTemp8 = _16_s / _21_A;
}
_2_g = _skTemp8;
}
} else {
_2_g = select(_6_k, _3_h, vec2<bool>(_11_n == 0.0));
}
}
}
localCoord = _2_g;
}
}
var devPosition: vec4<f32> = localToDevice * vec4<f32>(localCoord, 0.0, 1.0);
devPosition.z = _stageIn.depth;
(*_stageOut).sk_Position = vec4<f32>(_uniform0.u[0].zw * devPosition.xy - sign(_uniform0.u[0].zw) * devPosition.ww, devPosition.zw);
}
}
@vertex fn main(_stageIn: VSIn) -> VSOut {
var _stageOut: VSOut;
_skslMain(_stageIn, &_stageOut);
return _stageOut;
}
