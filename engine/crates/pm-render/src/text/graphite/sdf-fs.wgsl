// Generated from Skia chrome/m152 747cf654ed6f864f4402e7d6eb54a6ede60141e5.
// Skia, Copyright Google LLC, BSD-3-Clause. See pm-skia-path/LICENSE.
diagnostic(off, derivative_uniformity);
struct FSIn {
@location(0) @interpolate(flat, either) ssboIndices: u32,
@location(1) @interpolate(linear) perPrimitiveColor: vec4<f32>,
@location(2) unormTexCoords: vec2<f32>,
@location(3) textureCoords: vec2<f32>,
@location(4) texIndex: f32,
};
struct FSOut {
@location(0) sk_FragColor: vec4<f32>,
};
struct CombinedUniforms {
combinedUniformData: array<CombinedUniformData>,
};
@group(0) @binding(1) var<storage, read> _storage0 : CombinedUniforms;
struct IntrinsicUniforms {
viewport: vec4<f32>,
dstReadBounds: vec4<f32>,
};
@group(0) @binding(0) var<uniform> _uniform1 : IntrinsicUniforms;
fn sample_indexed_atlas_h4f2iZZZZ(a: vec2<f32>, b: i32, c_Texture: texture_2d<f32>, c_Sampler: sampler, d_Texture: texture_2d<f32>, d_Sampler: sampler, e_Texture: texture_2d<f32>, e_Sampler: sampler, f_Texture: texture_2d<f32>, f_Sampler: sampler) -> vec4<f32> {
{
switch b {
case 1 {
return textureSampleBias(d_Texture, d_Sampler, a, -0.475);
}
case 2 {
return textureSampleBias(e_Texture, e_Sampler, a, -0.475);
}
case 3 {
return textureSampleBias(f_Texture, f_Sampler, a, -0.475);
}
case default {
return textureSampleBias(c_Texture, c_Sampler, a, -0.475);
}
}
}
}
struct CombinedUniformData {
paintColor: vec4<f32>,
maskToDevice: mat4x4<f32>,
localToDevice: mat4x4<f32>,
atlasSizeInv: vec2<f32>,
gammaParams: vec2<f32>,
};
@group(0) @binding(2) var atlas0_Sampler: sampler;
@group(0) @binding(5) var atlas0_Texture: texture_2d<f32>;
@group(0) @binding(4) var atlas1_Sampler: sampler;
@group(0) @binding(7) var atlas1_Texture: texture_2d<f32>;
@group(0) @binding(6) var atlas2_Sampler: sampler;
@group(0) @binding(9) var atlas2_Texture: texture_2d<f32>;
@group(0) @binding(8) var atlas3_Sampler: sampler;
@group(0) @binding(10009) var atlas3_Texture: texture_2d<f32>;
var<private> bufferIndex: u32;
fn _skslMain(_stageIn: FSIn, _stageOut: ptr<function, FSOut>) {
{
bufferIndex = _stageIn.ssboIndices;
let outputColor: vec4<f32> = _stageIn.perPrimitiveColor;
let gammaParams: vec2<f32> = _storage0.combinedUniformData[bufferIndex].gammaParams;
var outputCoverage: vec4<f32> = vec4<f32>(1.0);
var _0_sdf_text_coverage_fn: vec4<f32>;
var _1_d: f32 = 7.96875 * (sample_indexed_atlas_h4f2iZZZZ(_stageIn.textureCoords, i32(_stageIn.texIndex), atlas0_Texture, atlas0_Sampler, atlas1_Texture, atlas1_Sampler, atlas2_Texture, atlas2_Sampler, atlas3_Texture, atlas3_Sampler).x - 0.5019608);
_1_d = _1_d - gammaParams.x;
var _2_e: vec2<f32> = vec2<f32>(dpdx(_1_d), dpdy(_1_d));
let _3_f: f32 = dot(_2_e, _2_e);
var _skTemp10: vec2<f32>;
if _3_f >= 0.0001 {
_skTemp10 = _2_e * inverseSqrt(_3_f);
} else {
_skTemp10 = vec2<f32>(0.7071);
}
_2_e = _skTemp10;
let _skTemp11 = dpdx(_stageIn.unormTexCoords);
let _skTemp12 = dpdy(_stageIn.unormTexCoords);
let _4_g: mat2x2<f32> = mat2x2<f32>(_skTemp11[0], _skTemp11[1], _skTemp12[0], _skTemp12[1]);
let _5_h: vec2<f32> = vec2<f32>(_4_g * vec2<f32>(_2_e));
let _6_i: f32 = 0.65 * length(_5_h);
if gammaParams.y > 0.0 {
_0_sdf_text_coverage_fn = vec4<f32>(saturate((_1_d + _6_i) / (2.0 * _6_i)));
} else {
_0_sdf_text_coverage_fn = vec4<f32>(smoothstep(-_6_i, _6_i, _1_d));
}
outputCoverage = _0_sdf_text_coverage_fn;
(*_stageOut).sk_FragColor = outputColor * outputCoverage;
}
}
@fragment fn main(_stageIn: FSIn) -> FSOut {
var _stageOut: FSOut;
_skslMain(_stageIn, &_stageOut);
return _stageOut;
}
