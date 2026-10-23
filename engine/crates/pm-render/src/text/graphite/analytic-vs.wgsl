// Generated from Skia chrome/m152 747cf654ed6f864f4402e7d6eb54a6ede60141e5.
// Skia, Copyright Google LLC, BSD-3-Clause. See pm-skia-path/LICENSE.
diagnostic(off, derivative_uniformity);
struct VSIn {
@builtin(vertex_index) sk_VertexID: u32,
};
struct VSOut {
@builtin(position) sk_Position: vec4<f32>,
@location(0) jacobian: vec4<f32>,
@location(1) edgeDistances: vec4<f32>,
@location(2) xRadii: vec4<f32>,
@location(3) yRadii: vec4<f32>,
@location(4) strokeParams: vec2<f32>,
@location(5) perPixelControl: vec2<f32>,
};
/* unsupported */ var<private> sk_PointSize: f32;
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
fn inverse_3x3f(m: mat3x3<f32>) -> mat3x3<f32> {
let a00 = m[0].x; let a01 = m[0].y; let a02 = m[0].z;
let a10 = m[1].x; let a11 = m[1].y; let a12 = m[1].z;
let a20 = m[2].x; let a21 = m[2].y; let a22 = m[2].z;
let b01 =  a22*a11 - a12*a21;
let b11 = -a22*a10 + a12*a20;
let b21 =  a21*a10 - a11*a20;
let det = a00*b01 + a01*b11 + a02*b21;
return mat3x3<f32>(b01, (-a22*a01 + a02*a21), ( a12*a01 - a02*a11),
b11, ( a22*a00 - a02*a20), (-a12*a00 + a02*a10),
b21, (-a21*a00 + a01*a20), ( a11*a00 - a01*a10)) * (1/det);
}
fn perp_f2f2(a: vec2<f32>) -> vec2<f32> {
{
return vec2<f32>(-a.y, a.x);
}
}
fn analytic_rrect_vertex_fn_f4If2f2fff4f4f4f4ff33f4f4f4f4f2f2f2(a: u32, b: vec2<f32>, c: vec2<f32>, d: f32, e: f32, f: vec4<f32>, g: vec4<f32>, h: vec4<f32>, i: vec4<f32>, j: f32, k: mat3x3<f32>, l: ptr<function, vec4<f32>>, m: ptr<function, vec4<f32>>, n: ptr<function, vec4<f32>>, o: ptr<function, vec4<f32>>, p: ptr<function, vec2<f32>>, q: ptr<function, vec2<f32>>, r: ptr<function, vec2<f32>>) -> vec4<f32> {
{
var w: f32 = 1.0;
let x: u32 = (a + 1u) % 4u;
let y: bool = i.z <= 0.0;
var z: bool = false;
var A: vec4<f32>;
var B: vec4<f32>;
var C: vec4<f32> = vec4<f32>(1.0);
var D: bool = false;
if f.x < -1.0 {
{
D = f.y > 0.0;
A = select(h.xzzx, h.xxzz, vec4<bool>(D));
B = h.yyww;
if f.y < 0.0 {
{
(*n) = -f - 2.0;
(*o) = g;
(*p) = vec2<f32>(0.0, 1.0);
}
} else {
{
(*n) = g;
(*o) = (*n);
(*p) = f.zw;
var _skTemp1: f32;
if (*p).y < 0.0 {
_skTemp1 = 0.414213568;
} else {
_skTemp1 = sign((*p).y);
}
w = _skTemp1;
}
}
}
} else {
if any((f > vec4<f32>(0.0))) {
{
A = h.xzzx;
B = h.yyww;
(*n) = f;
(*o) = g;
(*p) = vec2<f32>(0.0, -1.0);
}
} else {
{
A = g;
B = h;
C = -f;
(*n) = vec4<f32>(0.0);
(*o) = vec4<f32>(0.0);
(*p) = vec2<f32>(0.0, 1.0);
z = true;
}
}
}
var E: vec2<f32> = vec2<f32>((*n)[a], (*o)[a]);
if (a % 2u) != 0u {
E = E.yx;
}
var F: vec2<f32> = vec2<f32>(1.0);
if all((E > vec2<f32>(0.0))) {
{
w = 0.414213568;
F = E.yx;
}
}
var G: vec4<f32> = A - A.wxyz;
var H: vec4<f32> = B - B.wxyz;
let I: vec4<f32> = 1.0 / max(abs(G), max(abs(H), vec4<f32>(1.0)));
G = G * I;
H = H * I;
var J: vec4<f32> = G * G + H * H;
let K: vec4<f32> = sign(J);
var L: vec4<f32> = vec4<f32>(0.0);
var M: vec2<f32> = vec2<f32>((*p).x);
if any(K == vec4<f32>(0.0)) {
if all(K == vec4<f32>(0.0)) {
{
G = vec4<f32>(0.0, 1.0, 0.0, -1.0);
H = vec4<f32>(-1.0, 0.0, 1.0, 0.0);
J = vec4<f32>(1.0);
}
} else {
{
let N: bool = (((K.x + K.y) + K.z) + K.w) > 2.5;
let O: vec4<f32> = select(H.yzwx, G.yzwx, vec4<bool>(N));
let P: vec4<f32> = select(-G.yzwx, H.yzwx, vec4<bool>(N));
G = mix(O, G, K);
H = mix(P, H, K);
J = mix(J.yzwx, J, K);
C = mix(C.yzwx, C, K);
if (!N) && (w == 0.0) {
{
M = M * vec2<f32>(K[a], K[x]);
L = (K - 1.0) * (*p).x;
(*p).y = 1.0;
w = 1.0;
}
}
}
}
}
let N: vec4<f32> = inverseSqrt(J);
G = G * N;
H = H * N;
let O: vec2<f32> = -vec2<f32>(G[x], H[x]);
let P: vec2<f32> = vec2<f32>(G[a], H[a]);
var Q: vec2<f32>;
var R: bool = false;
if d < 0.0 {
if (i.w < 0.0) || ((e * i.z) != 0.0) {
R = true;
} else {
{
let S: f32 = i.w;
let T: vec2<f32> = E + (select(M, -M, vec2<bool>(y)));
if (w == 1.0) || any((T <= vec2<f32>(S))) {
Q = T - S;
} else {
Q = T * b - S * c;
}
}
}
} else {
Q = (E + M) * (b + w * b.yx);
}
if R {
Q = i.xy;
} else {
{
Q = Q - E;
Q = (vec2<f32>(A[a], B[a]) + O * Q.x) + P * Q.y;
}
}
(*m) = (H * (A - Q.x) - G * (B - Q.y)) + L;
let S: mat3x3<f32> = inverse_3x3f(k);
var T: vec3<f32> = k * vec3<f32>(Q, 1.0);
(*l) = vec4<f32>(S[0].xy - S[0].z * Q, S[1].xy - S[1].z * Q);
if z {
{
let U: vec4<f32> = -H * (S[0].x - S[0].z * A) + G * (S[0].y - S[0].z * B);
let V: vec4<f32> = -H * (S[1].x - S[1].z * A) + G * (S[1].y - S[1].z * B);
(*m) = (*m) * inverseSqrt(U * U + V * V);
(*m) = (*m) + (1.0 - C) * abs(T.z);
let W: bool = all(C == vec4<f32>(1.0)) && (dot(abs(G * G.yzwx + H * H.yzwx), vec4<f32>(1.0)) < 0.00024);
if W {
{
let X: vec2<f32> = (*m).xy + (*m).zw;
(*q).y = 1.0 + min(min(X.x, X.y), abs(T.z));
}
} else {
(*q).y = 1.0 + abs(T.z);
}
}
}
if (d > 0.0) && (T.z > 0.0) {
{
let U: mat2x2<f32> = mat2x2<f32>((*l)[0], (*l)[1], (*l)[2], (*l)[3]);
let V: vec2<f32> = vec2<f32>(C[a], C[x]) * c;
var W: vec2<f32> = ((F.x * V.x) * perp_f2f2(-P)) * U;
var X: vec2<f32> = ((F.y * V.y) * perp_f2f2(O)) * U;
let Y: bool = all(V != vec2<f32>(0.0));
if (w == 1.0) && Y {
{
W = normalize(W);
X = normalize(X);
if dot(W, X) < -0.8 {
{
let Z: f32 = sign(determinant(mat2x2<f32>(W[0], W[1], X[0], X[1])));
W = Z * perp_f2f2(W);
X = -Z * perp_f2f2(X);
}
}
}
}
T = vec3<f32>((T.xy + T.z * normalize(W + X)), T.z);
if z {
(*m) = (*m) - T.z;
} else {
(*q).y = -T.z;
}
}
} else {
if !z {
(*q).y = 0.0;
}
}
var _skTemp2: f32;
if e != 0.0 {
_skTemp2 = 1.0;
} else {
_skTemp2 = select(0.0, -1.0, y);
}
(*q).x = f32(_skTemp2);
if D {
let _skTemp3 = mat2x2<f32>(H.x, -H.y, -G.x, G.y) * mat2x2<f32>((*l)[0], (*l)[1], (*l)[2], (*l)[3]);
(*l) = vec4<f32>(_skTemp3[0], _skTemp3[1]);
}
(*r) = Q;
return vec4<f32>(T.xy, T.z * j, T.z);
}
}
const positions: array<vec2<f32>, 9> = array<vec2<f32>, 9>(vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0), vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0));
const normals: array<vec2<f32>, 9> = array<vec2<f32>, 9>(vec2<f32>(1.0, 0.0), vec2<f32>(0.707106769), vec2<f32>(0.707106769), vec2<f32>(0.0, 1.0), vec2<f32>(0.707106769), vec2<f32>(0.707106769), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0));
fn _skslMain(_stageIn: VSIn, _stageOut: ptr<function, VSOut>) {
{
let cornerID: u32 = u32(i32(_stageIn.sk_VertexID)) / 9u;
let t: u32 = u32(i32(_stageIn.sk_VertexID)) % 9u;
var _skTemp4: f32;
if t < 4u {
_skTemp4 = 1.0;
} else {
_skTemp4 = select(-1.0, 0.0, t < 6u);
}
let normalScale: f32 = f32(_skTemp4);
let centerWeight: f32 = f32(t == 8u);
var local: vec2<f32>;
var _skTemp5: vec4<f32>;
var _skTemp6: vec4<f32>;
var _skTemp7: vec4<f32>;
var _skTemp8: vec4<f32>;
var _skTemp9: vec2<f32>;
var _skTemp10: vec2<f32>;
var _skTemp11: vec2<f32>;
let _skTemp12 = _uniform0.mat0.xyz;
let _skTemp13 = _uniform0.mat1.xyz;
let _skTemp14 = _uniform0.mat2.xyz;
let _skTemp15 = analytic_rrect_vertex_fn_f4If2f2fff4f4f4f4ff33f4f4f4f4f2f2f2(cornerID, positions[t], normals[t], normalScale, centerWeight, _uniform0.xRadiiOrFlags, _uniform0.radiiOrQuadXs, _uniform0.ltrbOrQuadYs, _uniform0.R_center, 0.5, mat3x3<f32>(_skTemp12[0], _skTemp12[1], _skTemp12[2], _skTemp13[0], _skTemp13[1], _skTemp13[2], _skTemp14[0], _skTemp14[1], _skTemp14[2]), &_skTemp5, &_skTemp6, &_skTemp7, &_skTemp8, &_skTemp9, &_skTemp10, &_skTemp11);
(*_stageOut).jacobian = _skTemp5;
(*_stageOut).edgeDistances = _skTemp6;
(*_stageOut).xRadii = _skTemp7;
(*_stageOut).yRadii = _skTemp8;
(*_stageOut).strokeParams = _skTemp9;
(*_stageOut).perPixelControl = _skTemp10;
local = _skTemp11;
let devPosition: vec4<f32> = _skTemp15;
(*_stageOut).sk_Position = vec4<f32>(_uniform0.viewport.zw * devPosition.xy - sign(_uniform0.viewport.zw) * devPosition.ww, devPosition.zw);
}
}
@vertex fn main(_stageIn: VSIn) -> VSOut {
var _stageOut: VSOut;
_skslMain(_stageIn, &_stageOut);
return _stageOut;
}
