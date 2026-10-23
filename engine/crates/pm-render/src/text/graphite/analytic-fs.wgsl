// Generated from Skia chrome/m152 747cf654ed6f864f4402e7d6eb54a6ede60141e5.
// Skia, Copyright Google LLC, BSD-3-Clause. See pm-skia-path/LICENSE.
diagnostic(off, derivative_uniformity);
struct FSIn {
@builtin(position) sk_FragCoord: vec4<f32>,
@location(0) jacobian: vec4<f32>,
@location(1) edgeDistances: vec4<f32>,
@location(2) xRadii: vec4<f32>,
@location(3) yRadii: vec4<f32>,
@location(4) strokeParams: vec2<f32>,
@location(5) perPixelControl: vec2<f32>,
};
struct FSOut {
@location(0) sk_FragColor: vec4<f32>,
};
struct Params {
viewport: vec4<f32>,
xRadiiOrFlags: vec4<f32>,
radiiOrQuadXs: vec4<f32>,
ltrbOrQuadYs: vec4<f32>,
R_center: vec4<f32>,
mat0: vec4<f32>,
mat1: vec4<f32>,
mat2: vec4<f32>,
color: vec4<f32>,
};
@group(0) @binding(0) var<uniform> _uniform0 : Params;
fn K_Qvf2f22f2f2f2f2(a: ptr<function, vec2<f32>>, b: mat2x2<f32>, c: vec2<f32>, d: vec2<f32>, e: vec2<f32>, f: vec2<f32>) {
{
let g: vec2<f32> = f - d;
if all((g > vec2<f32>(0.0))) {
if all((f > vec2<f32>(0.0))) || ((c.x > 0.0) && (c.y < 0.0)) {
{
let _87_a: vec2<f32> = g * e;
let _88_e: vec2<f32> = 1.0 / (f * f + c.x * c.x);
let _89_g: vec2<f32> = _88_e * _87_a;
let _90_c: vec2<f32> = _89_g * b;
let _91_h: f32 = inverseSqrt(dot(_90_c, _90_c));
let _92_i: f32 = (0.5 * _91_h) * (dot(_87_a, _89_g) - 1.0);
let _93_j: f32 = ((f.x * c.x) * _88_e.x) * _91_h;
var h: vec2<f32> = vec2<f32>(_93_j - _92_i, _93_j + _92_i);
h.y = select(-h.y, 1.0, (f.x - c.x) <= 0.0);
(*a) = min((*a), h);
}
} else {
if c.y == 0.0 {
{
let _94_c: vec2<f32> = e * b;
let h: f32 = ((c.x - g.x) - g.y) * inverseSqrt(dot(_94_c, _94_c));
(*a).x = min((*a).x, h);
}
}
}
}
}
}
fn L_Qvf2f22f2f4f4f4(a: ptr<function, vec2<f32>>, b: mat2x2<f32>, c: vec2<f32>, e: vec4<f32>, f: vec4<f32>, g: vec4<f32>) {
{
var _skTemp1: vec2<f32> = (*a);
K_Qvf2f22f2f2f2f2(&_skTemp1, b, c, e.xy, vec2<f32>(-1.0), vec2<f32>(f.x, g.x));
(*a) = _skTemp1;
var _skTemp2: vec2<f32> = (*a);
K_Qvf2f22f2f2f2f2(&_skTemp2, b, c, e.zy, vec2<f32>(1.0, -1.0), vec2<f32>(f.y, g.y));
(*a) = _skTemp2;
var _skTemp3: vec2<f32> = (*a);
K_Qvf2f22f2f2f2f2(&_skTemp3, b, c, e.zw, vec2<f32>(1.0), vec2<f32>(f.z, g.z));
(*a) = _skTemp3;
var _skTemp4: vec2<f32> = (*a);
K_Qvf2f22f2f2f2f2(&_skTemp4, b, c, e.xw, vec2<f32>(-1.0, 1.0), vec2<f32>(f.w, g.w));
(*a) = _skTemp4;
}
}
fn _skslMain(_stageIn: FSIn, _stageOut: ptr<function, FSOut>) {
{
var _0_analytic_rrect_coverage_fn: vec4<f32>;
if _stageIn.perPixelControl.x > 0.0 {
_0_analytic_rrect_coverage_fn = vec4<f32>(1.0);
} else {
if _stageIn.perPixelControl.y > 1.0 {
{
let _1_h: vec2<f32> = min(_stageIn.edgeDistances.xy, _stageIn.edgeDistances.zw);
let _2_i: f32 = min(_1_h.x, _1_h.y) * _stageIn.sk_FragCoord.w;
let _3_j: f32 = (_stageIn.perPixelControl.y - 1.0) * _stageIn.sk_FragCoord.w;
let _4_k: f32 = 1.0 - 0.5 * _3_j;
_0_analytic_rrect_coverage_fn = vec4<f32>(f32(saturate(_3_j * (_2_i + _4_k))));
}
} else {
{
let _5_h: mat2x2<f32> = mat2x2<f32>(_stageIn.jacobian[0], _stageIn.jacobian[1], _stageIn.jacobian[2], _stageIn.jacobian[3]) * (1.0 / _stageIn.sk_FragCoord.w);
let _6_c: vec2<f32> = vec2<f32>(1.0, 0.0) * _5_h;
let _7_c: vec2<f32> = vec2<f32>(0.0, 1.0) * _5_h;
let _8_i: vec2<f32> = vec2<f32>(inverseSqrt(dot(_6_c, _6_c)), inverseSqrt(dot(_7_c, _7_c)));
let _9_j: vec2<f32> = _8_i * (_stageIn.strokeParams.x + min(_stageIn.edgeDistances.xy, _stageIn.edgeDistances.zw));
var _10_k: vec2<f32> = vec2<f32>(min(_9_j.x, _9_j.y), -1.0);
var _11_l: f32;
var _12_m: f32;
if _stageIn.perPixelControl.x > -0.95 {
{
let _13_n: vec2<f32> = _8_i * ((_stageIn.edgeDistances.xy + _stageIn.edgeDistances.zw) + 2.0 * _stageIn.strokeParams.xx);
_11_l = min(min(_13_n.x, _13_n.y), 1.0);
_12_m = 1.0 - 0.5 * _11_l;
}
} else {
{
let _14_n: vec2<f32> = (2.0 * _stageIn.strokeParams.x) * _8_i;
let _15_o: vec2<f32> = _14_n - _9_j;
_10_k.y = -max(_15_o.x, _15_o.y);
if _stageIn.strokeParams.x > 0.0 {
{
let _16_p: f32 = min(_14_n.x, _14_n.y);
let _17_q: vec2<f32> = select(vec2<f32>(_16_p), _14_n, (_15_o >= vec2<f32>(-0.5)));
_11_l = saturate(max(_17_q.x, _17_q.y));
_12_m = 1.0 - 0.5 * _11_l;
}
} else {
_12_m = 1.0;
_11_l = _12_m;
}
}
}
var _skTemp5: vec2<f32> = _10_k;
L_Qvf2f22f2f4f4f4(&_skTemp5, _5_h, _stageIn.strokeParams, _stageIn.edgeDistances, _stageIn.xRadii, _stageIn.yRadii);
_10_k = _skTemp5;
let _18_n: f32 = min(_stageIn.perPixelControl.y, 0.0) * _stageIn.sk_FragCoord.w;
let _19_o: f32 = _11_l * (min(_10_k.x + _18_n, -_10_k.y) + _12_m);
_0_analytic_rrect_coverage_fn = vec4<f32>(f32(saturate(_19_o)));
}
}
}
let coverage: vec4<f32> = _0_analytic_rrect_coverage_fn;
(*_stageOut).sk_FragColor = vec4<f32>(_uniform0.color) * coverage;
}
}
@fragment fn main(_stageIn: FSIn) -> FSOut {
var _stageOut: FSOut;
_skslMain(_stageIn, &_stageOut);
return _stageOut;
}
