// Generated from Skia chrome/m152 747cf654ed6f864f4402e7d6eb54a6ede60141e5.
// Skia, Copyright Google LLC, BSD-3-Clause. See pm-skia-path/LICENSE.
diagnostic(off, derivative_uniformity);
struct VSIn {
@builtin(vertex_index) sk_VertexID: u32,
@location(0) @interpolate(flat, either) size: vec2<u32>,
@location(1) @interpolate(flat, either) uvPos: vec2<u32>,
@location(2) xyPos: vec2<f32>,
@location(3) @interpolate(flat, either) indexAndFlags: vec2<u32>,
@location(4) strikeToSourceScale: f32,
@location(5) depth: f32,
@location(6) @interpolate(flat, either) ssboIndex: u32,
};
struct VSOut {
@builtin(position) sk_Position: vec4<f32>,
@location(0) @interpolate(flat, either) ssboIndices: u32,
@location(1) @interpolate(linear) perPrimitiveColor: vec4<f32>,
@location(2) unormTexCoords: vec2<f32>,
@location(3) textureCoords: vec2<f32>,
@location(4) texIndex: f32,
};
/* unsupported */ var<private> sk_PointSize: f32;
struct IntrinsicUniforms {
viewport: vec4<f32>,
dstReadBounds: vec4<f32>,
};
@group(0) @binding(0) var<uniform> _uniform0 : IntrinsicUniforms;
struct CombinedUniforms {
combinedUniformData: array<CombinedUniformData>,
};
@group(0) @binding(1) var<storage, read> _storage1 : CombinedUniforms;
fn inverse_4x4f(m: mat4x4<f32>) -> mat4x4<f32>{
let a00 = m[0].x; let a01 = m[0].y; let a02 = m[0].z; let a03 = m[0].w;
let a10 = m[1].x; let a11 = m[1].y; let a12 = m[1].z; let a13 = m[1].w;
let a20 = m[2].x; let a21 = m[2].y; let a22 = m[2].z; let a23 = m[2].w;
let a30 = m[3].x; let a31 = m[3].y; let a32 = m[3].z; let a33 = m[3].w;
let b00 = a00*a11 - a01*a10;
let b01 = a00*a12 - a02*a10;
let b02 = a00*a13 - a03*a10;
let b03 = a01*a12 - a02*a11;
let b04 = a01*a13 - a03*a11;
let b05 = a02*a13 - a03*a12;
let b06 = a20*a31 - a21*a30;
let b07 = a20*a32 - a22*a30;
let b08 = a20*a33 - a23*a30;
let b09 = a21*a32 - a22*a31;
let b10 = a21*a33 - a23*a31;
let b11 = a22*a33 - a23*a32;
let det = b00*b11 - b01*b10 + b02*b09 + b03*b08 - b04*b07 + b05*b06;
return mat4x4<f32>(a11*b11 - a12*b10 + a13*b09,
a02*b10 - a01*b11 - a03*b09,
a31*b05 - a32*b04 + a33*b03,
a22*b04 - a21*b05 - a23*b03,
a12*b08 - a10*b11 - a13*b07,
a00*b11 - a02*b08 + a03*b07,
a32*b02 - a30*b05 - a33*b01,
a20*b05 - a22*b02 + a23*b01,
a10*b10 - a11*b08 + a13*b06,
a01*b08 - a00*b10 - a03*b06,
a30*b04 - a31*b02 + a33*b00,
a21*b02 - a20*b04 - a23*b00,
a11*b07 - a10*b09 - a12*b06,
a00*b09 - a01*b07 + a02*b06,
a31*b01 - a30*b03 - a32*b00,
a20*b03 - a21*b01 + a22*b00) * (1/det);
}
fn text_vertex_fn_f4f2f44f44f2f2f2f2fff2f2f2(_skParam0: vec2<f32>, b: mat4x4<f32>, c: mat4x4<f32>, d: vec2<f32>, e: vec2<f32>, f: vec2<f32>, g: vec2<f32>, h: f32, i: f32, j: ptr<function, vec2<f32>>, k: ptr<function, vec2<f32>>, l: ptr<function, vec2<f32>>) -> vec4<f32> {
var a = _skParam0;
{
a = a * e;
let m: vec2<f32> = h * a + g;
let n: vec4<f32> = b * vec4<f32>(m, 0.0, 1.0);
(*l) = (inverse_4x4f(c) * n).xy;
(*k) = a + f;
(*j) = (*k) * d;
return vec4<f32>(n.xy, i * n.w, n.w);
}
}
struct CombinedUniformData {
paintColor: vec4<f32>,
maskToDevice: mat4x4<f32>,
localToDevice: mat4x4<f32>,
atlasSizeInv: vec2<f32>,
gammaParams: vec2<f32>,
};
var<private> bufferIndex: u32;
fn _skslMain(_stageIn: VSIn, _stageOut: ptr<function, VSOut>) {
{
var stepLocalCoords: vec2<f32> = vec2<f32>(0.0);
bufferIndex = _stageIn.ssboIndex;
let maskToDevice: mat4x4<f32> = _storage1.combinedUniformData[bufferIndex].maskToDevice;
let localToDevice: mat4x4<f32> = _storage1.combinedUniformData[bufferIndex].localToDevice;
let atlasSizeInv: vec2<f32> = _storage1.combinedUniformData[bufferIndex].atlasSizeInv;
(*_stageOut).texIndex = f32(f32(_stageIn.indexAndFlags.x));
var _skTemp2: vec2<f32>;
var _skTemp3: vec2<f32>;
var _skTemp4: vec2<f32>;
let _skTemp5 = text_vertex_fn_f4f2f44f44f2f2f2f2fff2f2f2(vec2<f32>(f32(i32(_stageIn.sk_VertexID) >> 1), f32(i32(_stageIn.sk_VertexID) & 1)), maskToDevice, localToDevice, atlasSizeInv, vec2<f32>(_stageIn.size), vec2<f32>(_stageIn.uvPos), _stageIn.xyPos, _stageIn.strikeToSourceScale, _stageIn.depth, &_skTemp2, &_skTemp3, &_skTemp4);
(*_stageOut).textureCoords = _skTemp2;
(*_stageOut).unormTexCoords = _skTemp3;
stepLocalCoords = _skTemp4;
let devPosition: vec4<f32> = _skTemp5;
(*_stageOut).sk_Position = vec4<f32>(_uniform0.viewport.zw * devPosition.xy - sign(_uniform0.viewport.zw) * devPosition.ww, devPosition.zw);
(*_stageOut).perPrimitiveColor = _storage1.combinedUniformData[bufferIndex].paintColor;
(*_stageOut).ssboIndices = bufferIndex;
}
}
@vertex fn main(_stageIn: VSIn) -> VSOut {
var _stageOut: VSOut;
_skslMain(_stageIn, &_stageOut);
return _stageOut;
}
