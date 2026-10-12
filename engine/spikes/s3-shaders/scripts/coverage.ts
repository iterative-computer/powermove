/*
 * GLSL ES 3.00 coverage sweep: one small program per built-in function (every
 * overload family in section 8 of the GLSL ES 3.00 specification), per sampler
 * type, and per language feature (types, operators, control flow, qualifiers,
 * preprocessor, identifiers that are reserved only in GLSL 4.50). The corpus
 * shows what Powermove shaders use today; this sweep shows what the pipeline
 * does with the rest of the language an author (or the in-app agent) may use.
 *
 * Every probe is a complete fragment program written against the compositor's
 * vertex shader and rendered as a one-pass effect over the `layer` test image
 * (u_tex on unit 0, the compositor's uniforms), so the normal runners compare
 * pixels as well as compile verdicts:
 *
 *   bun scripts/coverage.ts                         # → work/coverage/corpus.json
 *   bun scripts/webgl.ts --corpus work/coverage/corpus.json --out work/webgl/coverage-metal
 *   ./target/release/s3-shaders run --corpus work/coverage/corpus.json \
 *       --webgl work/webgl/coverage-metal --out work/coverage/results
 *   bun scripts/coverage.ts --summarize             # Markdown tables
 *
 * Results are mapped into [0, 1] with smooth functions (x / (1 + |x|)), never
 * `fract`, so rounding-level differences stay rounding-level in 8 bits.
 * Inputs avoid undefined behavior (domains of asin, log, pow, ...).
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const SPIKE = join(import.meta.dir, '..');
const WORK = join(SPIKE, 'work');
const OUT = join(WORK, 'coverage');
const args = process.argv.slice(2);

type Probe = { group: string; name: string; body: string; decls?: string; noTex?: boolean; out?: string; header?: string };
const probes: Probe[] = [];
const add = (group: string, name: string, body: string, extra: Partial<Probe> = {}) => probes.push({ group, name, body, ...extra });

const SHOW = `
vec4 show(float x){ x = x / (1.0 + abs(x)); return vec4(vec3(0.5 + 0.5 * x), 1.0); }
vec4 show(vec2 x){ x = x / (1.0 + abs(x)); return vec4(0.5 + 0.5 * x, 0.5, 1.0); }
vec4 show(vec3 x){ x = x / (1.0 + abs(x)); return vec4(0.5 + 0.5 * x, 1.0); }
vec4 show(vec4 x){ x = x / (1.0 + abs(x)); return 0.5 + 0.5 * x; }
vec4 show(int x){ return vec4(float(x & 255) / 255.0, float((x >> 8) & 255) / 255.0, float((x >> 16) & 255) / 255.0, 1.0); }
vec4 show(ivec2 x){ return (show(x.x) + show(x.y)) * 0.5; }
vec4 show(ivec3 x){ return (show(x.x) + show(x.y) + show(x.z)) * 0.25; }
vec4 show(ivec4 x){ return (show(x.x) + show(x.y) + show(x.z) + show(x.w)) * 0.25; }
vec4 show(uint x){ return vec4(float(x & 255u) / 255.0, float((x >> 8u) & 255u) / 255.0, float(x >> 24u) / 255.0, 1.0); }
vec4 show(uvec2 x){ return (show(x.x) + show(x.y)) * 0.5; }
vec4 show(uvec3 x){ return (show(x.x) + show(x.y) + show(x.z)) * 0.25; }
vec4 show(uvec4 x){ return (show(x.x) + show(x.y) + show(x.z) + show(x.w)) * 0.25; }
vec4 show(bool x){ return vec4(x ? 1.0 : 0.0, 0.0, 0.0, 1.0); }
vec4 show(bvec2 x){ return vec4(float(x.x), float(x.y), 0.0, 1.0); }
vec4 show(bvec3 x){ return vec4(float(x.x), float(x.y), float(x.z), 1.0); }
vec4 show(bvec4 x){ return vec4(float(x.x), float(x.y), float(x.z), float(x.w)); }
vec4 show(mat2 m){ return show(vec4(m[0], m[1])); }
vec4 show(mat3 m){ return (show(m[0]) + show(m[1]) + show(m[2])) * 0.25; }
vec4 show(mat4 m){ return (show(m[0]) + show(m[1]) + show(m[2]) + show(m[3])) * 0.25; }
vec4 show(mat2x3 m){ return (show(m[0]) + show(m[1])) * 0.5; }
vec4 show(mat2x4 m){ return (show(m[0]) + show(m[1])) * 0.5; }
vec4 show(mat3x2 m){ return (show(m[0]) + show(m[1]) + show(m[2])) * 0.25; }
vec4 show(mat3x4 m){ return (show(m[0]) + show(m[1]) + show(m[2])) * 0.25; }
vec4 show(mat4x2 m){ return (show(m[0]) + show(m[1]) + show(m[2]) + show(m[3])) * 0.25; }
vec4 show(mat4x3 m){ return (show(m[0]) + show(m[1]) + show(m[2]) + show(m[3])) * 0.25; }
`;

const PRELUDE = (tex: string) => `
  vec2 p = v_uv * 4.0 - 2.0;
  vec4 t = ${tex};
  float s = p.x + 0.37 * p.y;
  vec2 v2 = p;
  vec3 v3 = vec3(p, t.r - 0.5);
  vec4 v4 = vec4(p, t.r - 0.5, t.a);
  int i = int(floor(p.x * 37.0 + p.y * 11.0));
  ivec2 i2 = ivec2(floor(p * 37.0));
  ivec3 i3 = ivec3(i2, i * 3);
  ivec4 i4 = ivec4(i3, -i);
  uint u = uint(i + 100) * 2654435761u;
  uvec2 u2 = uvec2(u, u ^ 0x9e3779b9u);
  uvec3 u3 = uvec3(u2, u * 3u);
  uvec4 u4 = uvec4(u3, u >> 7u);
  bool b = p.x > p.y;
  bvec2 b2 = bvec2(p.x > 0.0, p.y > 0.0);
  bvec3 b3 = bvec3(b2, t.r > 0.5);
  bvec4 b4 = bvec4(b3, b);
  mat2 m2 = mat2(v4 + vec4(3.0, 0.0, 0.0, 3.0));
  mat3 m3 = mat3(v3 + vec3(5.0, 0.0, 0.0), v3.yzx + vec3(0.0, 5.0, 0.0), v3.zxy + vec3(0.0, 0.0, 5.0));
  mat4 m4 = mat4(v4 + vec4(6.0, 0.0, 0.0, 0.0), v4.yzwx + vec4(0.0, 6.0, 0.0, 0.0), v4.zwxy + vec4(0.0, 0.0, 6.0, 0.0), v4.wxyz + vec4(0.0, 0.0, 0.0, 6.0));
  mat2x3 m23 = mat2x3(v3, v3.yzx);
  mat2x4 m24 = mat2x4(v4, v4.yzwx);
  mat3x2 m32 = mat3x2(v2, v2.yx, v2 * 0.5);
  mat3x4 m34 = mat3x4(v4, v4.yzwx, v4.zwxy);
  mat4x2 m42 = mat4x2(v2, v2.yx, -v2, v2 * 0.3);
  mat4x3 m43 = mat4x3(v3, v3.yzx, v3.zxy, -v3);
`;

const HEADER = (out: string) => `#version 300 es
precision highp float;
in vec2 v_uv; in vec2 v_st; in vec2 v_px;
uniform sampler2D u_tex;
uniform vec2 u_res;
${out}
`;

function program(probe: Probe): string {
  const header = probe.header ?? HEADER(probe.out ?? 'out vec4 o;');
  return `${header}${probe.decls ?? ''}${SHOW}void main(){${PRELUDE(probe.noTex ? 'vec4(v_uv, 0.25, 1.0)' : 'texture(u_tex, v_uv)')}  ${probe.body}\n}\n`;
}

// Overload families. Each averages the overloads so every one is live.
const F = (f: (x: string) => string) => `o = (show(${f('s')}) + show(${f('v2')}) + show(${f('v3')}) + show(${f('v4')})) * 0.25;`;
const FI = (f: (x: string) => string) => `o = (show(${f('i')}) + show(${f('i2')}) + show(${f('i3')}) + show(${f('i4')})) * 0.25;`;
const FU = (f: (x: string) => string) => `o = (show(${f('u')}) + show(${f('u2')}) + show(${f('u3')}) + show(${f('u4')})) * 0.25;`;
const FB = (f: (x: string, b: string) => string) => `o = (show(${f('s', 'b')}) + show(${f('v2', 'b2')}) + show(${f('v3', 'b3')}) + show(${f('v4', 'b4')})) * 0.25;`;
const V = (f: (x: string) => string) => `o = (show(${f('v2')}) + show(${f('v3')}) + show(${f('v4')})) * 0.25;`;
const VI = (f: (x: string) => string) => `o = (show(${f('i2')}) + show(${f('i3')}) + show(${f('i4')})) * 0.25;`;
const VU = (f: (x: string) => string) => `o = (show(${f('u2')}) + show(${f('u3')}) + show(${f('u4')})) * 0.25;`;
const VB = (f: (x: string) => string) => `o = (show(${f('b2')}) + show(${f('b3')}) + show(${f('b4')})) * 0.25;`;
const NAN = { s: 'uintBitsToFloat(0x7fc00000u)', v2: 'vec2(uintBitsToFloat(0x7fc00000u))', v3: 'vec3(uintBitsToFloat(0x7fc00000u))', v4: 'vec4(uintBitsToFloat(0x7fc00000u))' } as Record<string, string>;
const INF = { s: 'uintBitsToFloat(0x7f800000u)', v2: 'vec2(uintBitsToFloat(0x7f800000u))', v3: 'vec3(uintBitsToFloat(0x7f800000u))', v4: 'vec4(uintBitsToFloat(0x7f800000u))' } as Record<string, string>;
const MATS = ['m2', 'm3', 'm4', 'm23', 'm24', 'm32', 'm34', 'm42', 'm43'];
const MSUM = (f: (m: string) => string, mats = MATS) => `o = (${mats.map((m) => `show(${f(m)})`).join(' + ')}) * ${(1 / mats.length).toFixed(6)};`;

// 8.1 Angle and trigonometry.
add('builtin-trig', 'radians', F((x) => `radians(${x} * 40.0)`));
add('builtin-trig', 'degrees', F((x) => `degrees(${x}) * 0.01`));
for (const fn of ['sin', 'cos', 'tan', 'sinh', 'cosh', 'tanh', 'asinh']) add('builtin-trig', fn, F((x) => `${fn}(${x})`));
add('builtin-trig', 'asin', F((x) => `asin(${x} * 0.3)`));
add('builtin-trig', 'acos', F((x) => `acos(${x} * 0.3)`));
add('builtin-trig', 'atan-y-over-x', F((x) => `atan(${x})`));
add('builtin-trig', 'atan-y-x', `o = (show(atan(s, 0.7)) + show(atan(v2, v2.yx)) + show(atan(v3, v3.zxy)) + show(atan(v4, v4.wzyx))) * 0.25;`);
add('builtin-trig', 'acosh', F((x) => `acosh(1.0 + abs(${x}))`));
add('builtin-trig', 'atanh', F((x) => `atanh(${x} * 0.3)`));

// 8.2 Exponential.
add('builtin-exp', 'pow', F((x) => `pow(abs(${x}) + 0.1, ${x} * 0.5)`));
add('builtin-exp', 'exp', F((x) => `exp(${x})`));
add('builtin-exp', 'log', F((x) => `log(abs(${x}) + 0.01)`));
add('builtin-exp', 'exp2', F((x) => `exp2(${x})`));
add('builtin-exp', 'log2', F((x) => `log2(abs(${x}) + 0.01)`));
add('builtin-exp', 'sqrt', F((x) => `sqrt(abs(${x}))`));
add('builtin-exp', 'inversesqrt', F((x) => `inversesqrt(abs(${x}) + 0.01)`));

// 8.3 Common.
for (const fn of ['abs', 'sign', 'floor', 'trunc', 'ceil', 'fract']) add('builtin-common', `${fn}-float`, F((x) => `${fn}(${x} * 3.0)`));
add('builtin-common', 'round', F((x) => `round(${x} * 3.0)`));
add('builtin-common', 'roundEven', F((x) => `roundEven(${x} * 3.0)`));
add('builtin-common', 'roundEven-ties', F((x) => `roundEven(floor(${x} * 8.0) + 0.5)`));
add('builtin-common', 'mod-scalar', F((x) => `mod(${x} * 3.0, 0.7)`));
add('builtin-common', 'mod-vector', F((x) => `mod(${x} * 3.0, abs(${x}) * 0.3 + 0.2)`));
add('builtin-common', 'modf', F((x) => `modf(${x} * 3.0, ${x})`)); // writes the whole part into the input
add('builtin-common', 'modf-both', `float fi; vec2 f2; vec3 f3; vec4 f4;
  o = (show(modf(s * 3.0, fi) + fi * 0.1) + show(modf(v2 * 3.0, f2) + f2 * 0.1) + show(modf(v3 * 3.0, f3) + f3 * 0.1) + show(modf(v4 * 3.0, f4) + f4 * 0.1)) * 0.25;`);
add('builtin-common', 'modf-statement', `vec2 whole; vec2 part = modf(v2 * 5.0, whole); o = show(vec4(part, whole * 0.1));`);
add('builtin-common', 'modf-infinity', `float w; float f = modf(b ? uintBitsToFloat(0x7f800000u) : s * 3.0, w); o = show(vec2(f, isinf(w) ? 9.0 : w));`);
add('builtin-common', 'min-max-float', F((x) => `min(${x}, ${x} * ${x} - 1.0) + max(${x}, 0.3) + min(${x}, 0.3) + max(${x}, ${x} * ${x} - 1.0)`));
add('builtin-common', 'clamp-float', F((x) => `clamp(${x}, -0.5, 0.8) + clamp(${x}, -abs(${x}) * 0.5, abs(${x}) * 0.5)`));
add('builtin-common', 'mix-float', F((x) => `mix(${x}, ${x} * ${x}, 0.3) + mix(${x}, -${x}, fract(${x}))`));
add('builtin-common', 'mix-bool', FB((x, bb) => `mix(${x}, ${x} * ${x}, ${bb})`));
add('builtin-common', 'step', F((x) => `step(0.2, ${x}) + step(${x} * ${x}, ${x})`));
add('builtin-common', 'smoothstep', F((x) => `smoothstep(-0.5, 0.8, ${x}) + smoothstep(-abs(${x}) - 0.1, abs(${x}) + 0.2, ${x} * 0.5)`));
add('builtin-common', 'isnan', FB((x, bb) => `isnan(mix(${x}, ${NAN[x]}, ${bb}))`));
add('builtin-common', 'isinf', FB((x, bb) => `isinf(mix(${x}, ${INF[x]}, ${bb}))`));
add('builtin-common', 'floatBitsToInt', F((x) => `floatBitsToInt(${x})`));
add('builtin-common', 'floatBitsToUint', F((x) => `floatBitsToUint(${x})`));
add('builtin-common', 'intBitsToFloat', FI((x) => `intBitsToFloat((${x} & 0x007fffff) | 0x3f800000)`));
add('builtin-common', 'uintBitsToFloat', FU((x) => `uintBitsToFloat((${x} & 0x007fffffu) | 0x3f800000u)`));
add('builtin-int', 'abs-sign-int', FI((x) => `abs(${x}) + sign(${x}) * 1000`));
add('builtin-int', 'min-max-clamp-int', FI((x) => `min(${x}, ${x} * 2 - 5) + max(${x}, 3) + clamp(${x}, -20, 20) + clamp(${x}, -abs(${x}) / 2, abs(${x}) / 2)`));
add('builtin-int', 'min-max-clamp-uint', FU((x) => `min(${x}, ${x} >> 3u) + max(${x}, 77u) + clamp(${x}, 1000u, 4000000000u)`));

// 8.4 Packing.
add('builtin-pack', 'packSnorm2x16', `o = show(packSnorm2x16(v2 * 0.6));`);
add('builtin-pack', 'unpackSnorm2x16', `o = show(unpackSnorm2x16(u));`);
add('builtin-pack', 'packUnorm2x16', `o = show(packUnorm2x16(v_uv));`);
add('builtin-pack', 'unpackUnorm2x16', `o = show(unpackUnorm2x16(u));`);
add('builtin-pack', 'packHalf2x16', `o = show(packHalf2x16(v2 * 3.1));`);
add('builtin-pack', 'unpackHalf2x16', `o = show(unpackHalf2x16(u & 0x3fff3fffu));`);

// 8.5 Geometric.
add('builtin-geometric', 'length', F((x) => `length(${x})`));
add('builtin-geometric', 'distance', F((x) => `distance(${x}, ${x} * ${x})`));
add('builtin-geometric', 'dot', F((x) => `dot(${x}, ${x} * ${x} - 0.3)`));
add('builtin-geometric', 'cross', `o = show(cross(v3, v3.zxy + 0.5));`);
add('builtin-geometric', 'normalize', F((x) => `normalize(${x} + 0.01)`));
add('builtin-geometric', 'faceforward', F((x) => `faceforward(${x}, ${x} * ${x} - 0.5, ${x} * 0.3 + 0.1)`));
add('builtin-geometric', 'reflect', F((x) => `reflect(${x}, normalize(${x} * ${x} + 0.1))`));
add('builtin-geometric', 'normalize-scalar-zero', `o = show(vec2(normalize(s * 0.0), normalize(-abs(s))));`);
add('builtin-geometric', 'scalar-geometric', `o = show(vec4(faceforward(s, s * s - 0.5, s * 0.3 + 0.1), reflect(s, 0.6), refract(0.5, -0.8, 0.66 + s * 0.1), refract(0.9, 0.95, 1.6)));`);
add('builtin-geometric', 'refract', F((x) => `refract(normalize(${x} + 0.01), normalize(${x} * ${x} + 0.1), 0.66)`));

// 8.6 Matrix.
add('builtin-matrix', 'matrixCompMult', MSUM((m) => `matrixCompMult(${m}, ${m})`));
add('builtin-matrix', 'transpose', MSUM((m) => `transpose(${m})`));
add('builtin-matrix', 'determinant', `o = (show(determinant(m2)) + show(determinant(m3) * 0.01) + show(determinant(m4) * 0.001)) * 0.333333;`);
add('builtin-matrix', 'inverse', `o = (show(inverse(m2)) + show(inverse(m3)) + show(inverse(m4))) * 0.333333;`);
add('builtin-matrix', 'outerProduct', `o = (show(outerProduct(v2, v2.yx)) + show(outerProduct(v3, v3)) + show(outerProduct(v4, v4)) + show(outerProduct(v3, v2)) + show(outerProduct(v2, v3))
    + show(outerProduct(v4, v2)) + show(outerProduct(v2, v4)) + show(outerProduct(v4, v3)) + show(outerProduct(v3, v4))) * 0.111111;`);
add('builtin-matrix', 'matrix-arithmetic', `o = (show(m2 * m2 + m2 * 0.5 - m2 / 3.0) + show(m3 * v3) + show(v4 * m4) + show(m23 * m32) + show(m32 * v3) + show(v2 * m42)
    + show(m34 * m43) + show(-m24) + show(m42 * m24) + show(m4 * m34)) * 0.1;`);
add('builtin-matrix', 'matrix-compound', `mat3 a = m3; a *= m3; a += m3; a -= mat3(0.5); a /= 2.0; vec3 w = v3; w *= a; mat2 c = m2; c++; --c; o = show(a) * 0.5 + show(w) * 0.25 + show(c) * 0.25;`);
add('builtin-matrix', 'matrix-construct', `o = (show(mat3(m4)) + show(mat4(m2)) + show(mat2(m3)) + show(mat2x3(m42)) + show(mat4x3(m2)) + show(mat2(s)) + show(mat3(v4, v4, s)) + show(mat4x2(m32))) * 0.125;`);
add('builtin-matrix', 'matrix-dynamic-index', `int k = int(v_uv.x * 2.99); int j = int(v_uv.y * 2.99); mat3 a = m3; a[k][j] = 7.0; o = show(a[k]) * 0.5 + show(m4[k + 1][j]) * 0.25 + show(m43[3 - k]) * 0.25;`);
add('builtin-matrix', 'matrix-equality', `o = show(bvec4(m3 == m3, m3 != mat3(m4), m2 == mat2(1.0), m43 != m43));`);

// 8.7 Vector relational.
for (const fn of ['lessThan', 'lessThanEqual', 'greaterThan', 'greaterThanEqual']) {
  add('builtin-relational', `${fn}-float`, V((x) => `${fn}(${x}, ${x}.yx${x === 'v2' ? '' : x === 'v3' ? 'x' : 'yx'})`));
  add('builtin-relational', `${fn}-int`, VI((x) => `${fn}(${x}, ${x}.yx${x === 'i2' ? '' : x === 'i3' ? 'x' : 'yx'})`));
  add('builtin-relational', `${fn}-uint`, VU((x) => `${fn}(${x}, ${x}.yx${x === 'u2' ? '' : x === 'u3' ? 'x' : 'yx'})`));
}
for (const fn of ['equal', 'notEqual']) {
  add('builtin-relational', `${fn}-float`, V((x) => `${fn}(floor(${x} * 2.0), floor(${x}.yx${x === 'v2' ? '' : x === 'v3' ? 'x' : 'yx'} * 2.0))`));
  add('builtin-relational', `${fn}-int`, VI((x) => `${fn}(${x} / 16, ${x}.yx${x === 'i2' ? '' : x === 'i3' ? 'x' : 'yx'} / 16)`));
  add('builtin-relational', `${fn}-uint`, VU((x) => `${fn}(${x} >> 30u, ${x}.yx${x === 'u2' ? '' : x === 'u3' ? 'x' : 'yx'} >> 30u)`));
  add('builtin-relational', `${fn}-bool`, VB((x) => `${fn}(${x}, ${x}.yx${x === 'b2' ? '' : x === 'b3' ? 'x' : 'yx'})`));
}
add('builtin-relational', 'any-all-not', VB((x) => `bvec2(any(${x}), all(not(${x})))`));

// 8.8 Texture lookup on sampler2D (the type the compositor binds).
const T = (body: string) => `o = ${body};`;
add('builtin-texture', 'texture', T('texture(u_tex, v_uv)'));
add('builtin-texture', 'texture-bias', T('texture(u_tex, v_uv, 1.5)'));
add('builtin-texture', 'textureProj-vec3', T('textureProj(u_tex, vec3(v_uv * 1.7, 1.7))'));
add('builtin-texture', 'textureProj-vec4', T('textureProj(u_tex, vec4(v_uv * 1.7, 0.3, 1.7))'));
add('builtin-texture', 'textureLod', T('textureLod(u_tex, v_uv * 1.3, 0.0) + textureLod(u_tex, v_uv, 2.5)'));
add('builtin-texture', 'textureOffset', T('textureOffset(u_tex, v_uv, ivec2(3, -2))'));
add('builtin-texture', 'textureOffset-bias', T('textureOffset(u_tex, v_uv, ivec2(-8, 7), 0.5)'));
add('builtin-texture', 'texelFetch', T('texelFetch(u_tex, ivec2(gl_FragCoord.xy * 0.75), 0)'));
add('builtin-texture', 'texelFetchOffset', T('texelFetchOffset(u_tex, ivec2(gl_FragCoord.xy * 0.75), 0, ivec2(2, -3))'));
add('builtin-texture', 'textureProjOffset', T('textureProjOffset(u_tex, vec3(v_uv * 1.7, 1.7), ivec2(1, 1))'));
add('builtin-texture', 'textureLodOffset', T('textureLodOffset(u_tex, v_uv, 0.0, ivec2(-1, 4))'));
add('builtin-texture', 'textureProjLod', T('textureProjLod(u_tex, vec4(v_uv * 1.7, 0.3, 1.7), 0.0)'));
add('builtin-texture', 'textureProjLodOffset', T('textureProjLodOffset(u_tex, vec3(v_uv * 1.7, 1.7), 0.0, ivec2(2, 0))'));
add('builtin-texture', 'textureGrad', T('textureGrad(u_tex, v_uv, dFdx(v_uv), dFdy(v_uv))'));
add('builtin-texture', 'textureGradOffset', T('textureGradOffset(u_tex, v_uv, dFdx(v_uv), dFdy(v_uv), ivec2(0, 3))'));
add('builtin-texture', 'textureProjGrad', T('textureProjGrad(u_tex, vec3(v_uv * 1.7, 1.7), dFdx(v_uv), dFdy(v_uv))'));
add('builtin-texture', 'textureProjGradOffset', T('textureProjGradOffset(u_tex, vec4(v_uv * 1.7, 0.0, 1.7), dFdx(v_uv), dFdy(v_uv), ivec2(-2, 1))'));
add('builtin-texture', 'textureSize', `ivec2 z = textureSize(u_tex, 0); o = show(vec2(z) / 300.0);`);
add('builtin-texture', 'sampler-function-argument', `o = tap(u_tex, v_uv.yx);`, { decls: 'vec4 tap(sampler2D s, vec2 uv){ return texture(s, uv); }\n' });
add('builtin-texture', 'sampler-in-loop', `vec4 acc = vec4(0.0); for (int k = 0; k < 8; k++) acc += texture(u_tex, v_uv + vec2(float(k) * 0.01, 0.0)); o = acc / 8.0;`);

// Arrays of sampler2D (constant indices, as GLSL ES 3.00 requires).
add('samplers', 'sampler2D-array-constant-index', `o = texture(u_extra[0], v_uv) * 0.5 + texture(u_extra[1], v_uv.yx) * 0.5;`, { decls: 'uniform sampler2D u_extra[2];\n' });
add('samplers', 'sampler2D-array-const-expression', `const int K = 2; o = texture(u_extra[K - 1], v_uv) + texture(u_extra[K * 2 - 2], v_uv * 0.5);`, { decls: 'uniform sampler2D u_extra[3];\n' });
add('samplers', 'sampler2D-array-function-argument', `o = tap(u_extra[1], v_uv);`, { decls: 'uniform sampler2D u_extra[2];\nvec4 tap(sampler2D s, vec2 uv){ return texture(s, uv * 0.8); }\n' });
add('samplers', 'sampler2D-array-unused-element', `o = texture(u_extra[1], v_uv);`, { decls: 'uniform sampler2D u_extra[4];\n' });
// Other sampler types. The compositor binds only 2D RGBA16F textures, so on
// WebGL these default to unit 0, where u_tex's texture is: WebGL rejects a draw
// with two sampler types on one unit, or an integer sampler on a float
// texture. The no-u_tex forms leave only the unusual sampler on unit 0.
const SAMPLERS: [string, string, string][] = [
  ['sampler3D', 'texture(SMP, vec3(v_uv, 0.5))', 'vec4'],
  ['samplerCube', 'texture(SMP, vec3(p, 1.0))', 'vec4'],
  ['sampler2DArray', 'texture(SMP, vec3(v_uv, 0.0))', 'vec4'],
  ['sampler2DShadow', 'vec4(texture(SMP, vec3(v_uv, 0.5)))', 'float'],
  ['samplerCubeShadow', 'vec4(texture(SMP, vec4(p, 1.0, 0.5)))', 'float'],
  ['sampler2DArrayShadow', 'vec4(texture(SMP, vec4(v_uv, 0.0, 0.5)))', 'float'],
  ['isampler2D', 'vec4(texture(SMP, v_uv)) / 255.0', 'ivec4'],
  ['usampler2D', 'vec4(texture(SMP, v_uv)) / 255.0', 'uvec4'],
  ['isampler3D', 'vec4(texture(SMP, vec3(v_uv, 0.5))) / 255.0', 'ivec4'],
  ['usamplerCube', 'vec4(texture(SMP, vec3(p, 1.0))) / 255.0', 'uvec4'],
  ['isampler2DArray', 'vec4(texture(SMP, vec3(v_uv, 0.0))) / 255.0', 'ivec4'],
];
for (const [type, sample] of SAMPLERS) {
  add('samplers', `${type}-alone`, `o = ${sample.replace('SMP', 'u_s')};`, { decls: `uniform highp ${type} u_s;\n`, noTex: true });
  add('samplers', `${type}-with-u_tex`, `o = t * 0.5 + ${sample.replace('SMP', 'u_s')} * 0.5;`, { decls: `uniform highp ${type} u_s;\n` });
}
add('samplers', 'isampler2D-texelFetch-alone', `o = vec4(texelFetch(u_s, ivec2(gl_FragCoord.xy), 0)) / 255.0;`, { decls: 'uniform highp isampler2D u_s;\n', noTex: true });
add('samplers', 'isampler2D-textureSize-alone', `o = show(vec2(textureSize(u_s, 0)));`, { decls: 'uniform highp isampler2D u_s;\n', noTex: true });
add('samplers', 'isampler2D-declared-unused', `o = t;`, { decls: 'uniform highp isampler2D u_s;\n' });
add('samplers', 'sampler3D-declared-unused', `o = t;`, { decls: 'uniform highp sampler3D u_s;\n' });

// 8.9 Fragment processing.
add('builtin-derivative', 'dFdx', F((x) => `dFdx(${x} * ${x}) * 50.0`));
add('builtin-derivative', 'dFdy', F((x) => `dFdy(${x} * ${x}) * 50.0`));
add('builtin-derivative', 'fwidth', F((x) => `fwidth(${x} * ${x}) * 50.0`));

// Built-in variables and outputs.
add('variables', 'gl_FragCoord', `o = show(gl_FragCoord * 0.01);`);
add('variables', 'gl_FrontFacing', `o = show(gl_FrontFacing);`);
add('variables', 'gl_PointCoord', `o = show(gl_PointCoord) * 0.0 + t;`); // undefined for triangles: only compile matters
add('variables', 'gl_FragDepth', `gl_FragDepth = 0.25; o = t;`);
add('variables', 'output-vec3', `o = t.rgb;`, { out: 'out vec3 o;' });
add('variables', 'output-float', `o = t.g;`, { out: 'out float o;' });
add('variables', 'output-layout-location', `o = t;`, { out: 'layout(location = 0) out vec4 o;' });
add('variables', 'output-second-location', `o = t; o2 = vec4(1.0);`, { out: 'layout(location = 0) out vec4 o;\nlayout(location = 1) out vec4 o2;' });
add('variables', 'output-ivec4', `o = ivec4(t * 255.0);`, { out: 'out ivec4 o;' });
add('variables', 'output-array', `o[0] = t;`, { out: 'out vec4 o[1];' });
add('variables', 'gl_MaxDrawBuffers-constants', `o = show(vec4(float(gl_MaxDrawBuffers), float(gl_MaxTextureImageUnits), float(gl_MaxFragmentUniformVectors), float(gl_MinProgramTexelOffset)) * 0.01);`);
add('variables', 'uniform-block', `o = t * k + c;`, { decls: 'uniform Params { float k; vec4 c; };\n' });
add('variables', 'uniform-block-std140-instance', `o = t * params.k + params.c;`, { decls: 'layout(std140) uniform Params { float k; vec4 c; } params;\n' });
add('variables', 'uniform-struct', `o = t * st.k + st.c;`, { decls: 'struct P { float k; vec4 c; };\nuniform P st;\n' });
add('variables', 'uniform-array-dynamic-index', `o = t + ua[int(v_uv.x * 3.99)];`, { decls: 'uniform vec4 ua[4];\n' });
add('variables', 'uniform-int-uint-bool', `o = t * float(ui) + vec4(float(uu), float(ub), 0.0, 0.0);`, { decls: 'uniform int ui; uniform uint uu; uniform bool ub;\n' });
add('variables', 'uniform-matrices', `o = vec4(um2 * v2, 0.0, 1.0) + vec4(um3 * v3, 1.0) + um4 * v4 + vec4(um23 * v2, 0.0);`, { decls: 'uniform mat2 um2; uniform mat3 um3; uniform mat4 um4; uniform mat2x3 um23;\n' });

// Types.
add('types', 'uint-literals', `uint h = 0xFFFFFFFFu - 017u + 255u; o = show(h * u);`);
add('types', 'int-literals', `int a = 0x7fffffff; int c = 017; int d = -2147483647 - 1; o = show(ivec3(a + i, c, d / 3));`);
add('types', 'float-literals', `float a = 1e-3 + 2.5E2 + .5 + 5. + 1.0e+1 + 3.0f + 2.0F; o = show(a * s * 0.01);`);
add('types', 'conversions', `o = (show(int(s * 10.0)) + show(uint(abs(s) * 1000.0)) + show(float(i) * 0.1) + show(float(u) * 1e-9) + show(bool(i)) + show(int(b)) + show(uint(b2.x)) + show(ivec2(u2)) + show(uvec3(i3)) + show(vec4(b4)) + show(bvec2(i2)) + show(int(-2.7))) * 0.0833;`);
add('types', 'swizzles', `vec4 w = v4; w.zx = w.xz; w.yw += v2; vec2 r = w.rg + w.st + w.ba; ivec3 q = i4.wzy; o = show(vec4(r, w.ww)) * 0.5 + show(q) * 0.5;`);
add('types', 'arrays', `float a[5]; for (int k = 0; k < 5; k++) a[k] = s * float(k); float c[5] = a; c[2] = 1.0; o = show(vec4(a[4], c[2], float(a.length()), a[int(v_uv.x * 4.99)]));`);
add('types', 'array-constructors', `float a[3] = float[3](s, s * 2.0, s * 3.0); vec2 c[] = vec2[](v2, v2.yx); o = show(vec4(a[1] + a[2], c[1], float(c.length())));`);
add('types', 'array-equality', `float a[2] = float[2](s, 1.0); float c[2] = float[2](s, 1.0); o = show(bvec2(a == c, a != float[2](0.0, 1.0)));`);
add('types', 'const-arrays', `const vec3 pal[4] = vec3[4](vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0), vec3(1.0)); o = vec4(pal[int(v_uv.x * 3.99)], 1.0);`);
add('types', 'global-const-array', `o = vec4(K[int(v_uv.y * 2.99)] * t.rgb, 1.0);`, { decls: 'const float K[3] = float[3](0.25, 0.5, 1.0);\n' });
add('types', 'global-arrays-private', `G[int(v_uv.x * 7.99)] = t; o = G[int(v_uv.y * 7.99)];`, { decls: 'vec4 G[8];\n' });
add('types', 'structs', `S x = S(s, v3, float[2](1.0, 2.0)); x.c[1] = s; S y = x; y.a += 1.0; o = show(vec4(y.v, y.a + y.c[1])) * 0.5 + show(x == y) * 0.5;`, { decls: 'struct S { float a; vec3 v; float c[2]; };\n' });
add('types', 'nested-structs', `Outer w; w.inner = Inner(v2, i); w.k[1] = w.inner; w.k[0] = Inner(v2.yx, -i); o = show(vec4(w.k[1].q + w.k[0].q, float(w.k[0].n), 0.0)) * 0.5 + show(w.inner.n) * 0.5;`, { decls: 'struct Inner { vec2 q; int n; };\nstruct Outer { Inner inner; Inner k[2]; };\n' });
add('types', 'struct-function-io', `S x = make(s); bump(x); o = show(vec2(x.a, x.b));`, { decls: 'struct S { float a; float b; };\nS make(float v){ return S(v, v * 2.0); }\nvoid bump(inout S x){ x.a += 1.0; x.b *= 0.5; }\n' });
add('types', 'array-function-io', `float a[3]; fill(a, s); o = show(vec3(a[0], a[1], sum(a)));`, { decls: 'void fill(out float a[3], float v){ for (int k = 0; k < 3; k++) a[k] = v * float(k); }\nfloat sum(float a[3]){ return a[0] + a[1] + a[2]; }\n' });
add('types', 'struct-declared-in-function', `struct L { vec2 a; }; L l = L(v2); o = show(l.a);`);
add('types', 'vector-length-method', `o = show(vec3(float(v3.length()), float(m3.length()), float(i4.length())));`);
add('types', 'array-of-arrays', `float a[2][2]; a[0][1] = s; o = show(a[0][1]);`); // GLSL ES 3.10; WebGL2 refuses
add('types', 'bool-ops', `bool c = b ^^ b2.x; bool d = !(b && b2.y) || c; o = show(bvec3(c, d, b == d));`);

// Operators.
add('operators', 'int-arithmetic', FI((x) => `(${x} * 7 + 3) / 4 - ${x} % 5`));
add('operators', 'int-negative-division', FI((x) => `(${x} - 50) / 7 * 1000 + (${x} - 50) % 7`)); // % of negatives is undefined in ES 3.00
add('operators', 'uint-arithmetic', FU((x) => `(${x} * 7u + 3u) / 4u - ${x} % 5u`));
add('operators', 'bitwise-int', FI((x) => `((${x} << 3) ^ (${x} >> 1)) & ~(${x} | 12)`));
add('operators', 'bitwise-uint', FU((x) => `((${x} << 13u) ^ (${x} >> 7u)) & ~(${x} | 12u)`));
add('operators', 'shift-mixed-signedness', `o = show(ivec2(i << 3u, i >> 2u)) * 0.5 + show(uvec2(u << 3, u >> 5)) * 0.5;`);
add('operators', 'vector-scalar-mixed', `o = show(v3 * 2.0 + 1.0 - v3 / 3.0) * 0.5 + show(i3 * 2 + 1) * 0.25 + show(u2 / 3u) * 0.25;`);
add('operators', 'increment-decrement', `vec3 w = v3; w++; ++w; w--; int k = i; k++; ++k; float f = s; f--; o = show(vec4(w, f)) * 0.5 + show(k) * 0.5;`);
add('operators', 'compound-assign', `int k = i; k += 3; k -= 1; k *= 2; k /= 3; k %= 7; k <<= 2; k >>= 1; k &= 255; k |= 3; k ^= 5; uint w = u; w %= 1000u; w <<= 2u; o = show(k) * 0.5 + show(w) * 0.5;`);
add('operators', 'ternary-and-comma', `float a = b ? s : -s; vec3 w = b2.x ? v3 : v3.zyx; float c; c = (a += 1.0, a * 2.0); o = show(vec4(w, c));`);
add('operators', 'ternary-struct', `S x = b ? S(s) : S(-s); o = show(x.a);`, { decls: 'struct S { float a; };\n' });
add('operators', 'ternary-array', `float a[2] = b ? float[2](s, 1.0) : float[2](0.0, s); o = show(a[0] + a[1]);`);
add('operators', 'dynamic-vector-index', `vec4 w = v4; int k = int(v_uv.x * 3.99); w[k] = 9.0; o = show(vec2(w[k], v4[3 - k]));`);
add('operators', 'dynamic-array-lvalue', `float a[8]; for (int k = 0; k < 8; k++) a[k] = 0.0; a[int(t.r * 7.99)] += 1.0; a[int(v_uv.x * 7.99)] = s; o = show(vec4(a[0], a[3], a[7], a[int(v_uv.y * 7.99)]));`);
add('operators', 'short-circuit-side-effects', `float c = 0.0; bool r = b && (c += 1.0) > 0.0; bool q = b2.x || (c += 2.0) > 0.0; o = show(vec3(c, float(r), float(q)));`);
add('operators', 'integer-overflow-wrap', `int a = 2147483647; a += int(v_uv.x * 2.0) + 1; uint c = 0u; c -= uint(v_uv.y * 2.0) + 1u; o = show(ivec2(a, a >> 16)) * 0.5 + show(c) * 0.5;`);
add('operators', 'unary-minus-uint', `uint a = -u; o = show(a);`);

// Control flow.
add('control', 'switch', `int k = int(v_uv.x * 6.0); float r = 0.0; switch (k) { case 0: r = 0.1; break; case 1: case 2: r = 0.4; break; case 3: r = 0.6; default: r += 0.3; break; } o = show(r);`);
add('control', 'switch-uint-nested', `float r = 0.0; switch (u % 4u) { case 0u: switch (i & 1) { case 0: r = 1.0; break; default: r = 2.0; } break; case 1u: r = 3.0; break; default: r = 4.0; } o = show(r);`);
add('control', 'switch-empty-default-fallthrough', `float r = 1.0; switch (i & 3) { case 0: r = 2.0; case 1: r *= 3.0; default: break; } o = show(r);`);
add('control', 'do-while', `float a = s; int k = 0; do { a = a * 0.5 + 0.1; k++; } while (k < 5 && a > 0.0); o = show(vec2(a, float(k)));`);
add('control', 'while-break-continue', `float a = 0.0; int k = 0; while (true) { k++; if (k > 20) break; if ((k & 1) == 0) continue; a += float(k) * t.r; } o = show(a * 0.1);`);
add('control', 'for-float-counter', `float a = 0.0; for (float x = 0.0; x < 1.0; x += 0.125) a += sin(x + s); o = show(a);`);
add('control', 'for-dynamic-bound', `float a = 0.0; int n = int(v_uv.x * 20.0); for (int k = 0; k < n; k++) a += 0.05; o = show(a);`);
add('control', 'nested-loops-break', `float a = 0.0; for (int x = 0; x < 6; x++) { for (int y = 0; y < 6; y++) { if (y > x) break; a += float(x * y); } if (a > 40.0 * v_uv.x) break; } o = show(a * 0.01);`);
add('control', 'early-return-main', `o = t; if (v_uv.x > 0.5) return; o = 1.0 - t;`);
add('control', 'discard-in-main', `if (t.a < 0.5) discard; o = t;`);
add('control', 'discard-in-function', `kill(t.a); o = t;`, { decls: 'void kill(float a){ if (a < 0.5) discard; }\n' });
add('control', 'function-prototypes', `o = show(f(s));`, { decls: 'float g(float x);\nfloat f(float x){ return g(x) * 2.0; }\nfloat g(float x){ return x + 1.0; }\n' });
add('control', 'overloads', `o = show(h(s)) * 0.5 + show(h(v2)) * 0.5;`, { decls: 'float h(float x){ return x * 2.0; }\nvec2 h(vec2 x){ return x.yx; }\n' });
add('control', 'const-in-params', `o = show(f(s, v2));`, { decls: 'float f(const in float a, in vec2 c){ return a * c.x; }\n' });
add('control', 'out-param-aliasing', `float a = s; f(a, a); o = show(a);`, { decls: 'void f(in float x, out float y){ y = x * 2.0 + 1.0; }\n' });
add('control', 'inout-swizzle-argument', `vec3 w = v3; f(w.zx); o = show(w);`, { decls: 'void f(inout vec2 c){ c = c * 2.0 + 1.0; }\n' });
add('control', 'void-function-early-return', `float a = 0.0; f(a, s); o = show(a);`, { decls: 'void f(inout float a, float x){ if (x < 0.0) return; a = x; }\n' });
add('control', 'global-initializer-uniform', `o = t * g;`, { decls: 'float g = u_res.x / 512.0;\n' }); // non-constant global initializer: WebGL2 refuses
add('control', 'global-mutable', `counter += s; o = show(counter);`, { decls: 'float counter = 1.0;\n' });
add('control', 'shadowed-names', `float s2 = s; { float s = 2.0; s2 += s; } o = show(s2);`);

// Qualifiers and precision.
add('qualifiers', 'precision-mediump', `mediump float a = s * 1.0001; lowp vec4 c = t; o = c * 0.5 + show(a) * 0.5;`);
add('qualifiers', 'precision-statements', `o = t;`, { header: `#version 300 es\nprecision mediump float;\nprecision highp int;\nprecision lowp sampler2D;\nin vec2 v_uv; in vec2 v_st; in vec2 v_px;\nuniform sampler2D u_tex;\nuniform highp vec2 u_res;\nout vec4 o;\n` });
add('qualifiers', 'invariant-output', `o = t;`, { out: 'out vec4 o;\ninvariant o;' }); // fragment outputs cannot be invariant in ES 3.00
add('qualifiers', 'flat-input-unwritten', `o = t + vec4(float(nope));`, { decls: 'flat in int nope;\n' }); // vertex shader does not write it
add('qualifiers', 'centroid-input-unwritten', `o = t;`, { decls: 'centroid in vec2 nope;\n' }); // declared, unused
add('qualifiers', 'const-global-expression', `o = t * K;`, { decls: 'const float K = sqrt(4.0) * 0.25 + float(3 / 2);\n' });

// Preprocessor.
add('preprocessor', 'function-macros', `o = show(SQ(s) + ADD(s, 1.0));`, { decls: '#define SQ(x) ((x) * (x))\n#define ADD(a, b) ((a) + (b))\n' });
add('preprocessor', 'conditionals', `o = show(float(V));`, { decls: '#if defined(GL_ES) && __VERSION__ == 300\n#define V 1\n#elif 0\n#define V 2\n#else\n#define V 3\n#endif\n#ifdef GL_FRAGMENT_PRECISION_HIGH\n#undef V\n#define V 4\n#endif\n' });
add('preprocessor', 'line-macro', `o = show(float(__LINE__) * 0.01);`);
add('preprocessor', 'line-directive', `o = show(float(__LINE__) * 0.01);`, { decls: '#line 100\n' });
add('preprocessor', 'token-pasting', `float ab = 1.5; o = show(CAT(a, b));`, { decls: '#define CAT(x, y) x ## y\n' });
add('preprocessor', 'pragmas', `o = t;`, { decls: '#pragma optimize(off)\n#pragma debug(on)\n#pragma STDGL invariant(all)\n' });
add('preprocessor', 'error-in-dead-branch', `o = t;`, { decls: '#if 0\n#error unreachable\n#endif\n' });
add('preprocessor', 'error-directive', `o = t;`, { decls: '#error refused on purpose\n' });
add('preprocessor', 'macro-name-double-underscore', `o = t;`, { decls: '#define FOO__BAR 1\n' });
add('preprocessor', 'macro-name-GL_', `o = t;`, { decls: '#define GL_FOO 1\n' });
add('preprocessor', 'undef-GL_ES', `o = t;`, { decls: '#undef GL_ES\n' });
add('preprocessor', 'line-continuation', `float a = s \\\n  * 2.0; o = show(a);`);
add('preprocessor', 'undefined-macro-in-if', `o = show(float(X));`, { decls: '#if UNDEFINED_THING\n#define X 1\n#else\n#define X 2\n#endif\n' });
add('preprocessor', 'extension-standard-derivatives', `o = show(dFdx(s) * 50.0);`, { header: `#version 300 es\n#extension GL_OES_standard_derivatives : enable\nprecision highp float;\nin vec2 v_uv; in vec2 v_st; in vec2 v_px;\nuniform sampler2D u_tex;\nuniform vec2 u_res;\nout vec4 o;\n` });
add('preprocessor', 'extension-unknown-warn', `o = t;`, { header: `#version 300 es\n#extension GL_EXT_not_a_real_extension : warn\nprecision highp float;\nin vec2 v_uv; in vec2 v_st; in vec2 v_px;\nuniform sampler2D u_tex;\nuniform vec2 u_res;\nout vec4 o;\n` });
add('preprocessor', 'extension-unknown-require', `o = t;`, { header: `#version 300 es\n#extension GL_EXT_not_a_real_extension : require\nprecision highp float;\nin vec2 v_uv; in vec2 v_st; in vec2 v_px;\nuniform sampler2D u_tex;\nuniform vec2 u_res;\nout vec4 o;\n` });

// Identifiers: words that are keywords, reserved words, or built-in functions
// in GLSL 4.50 or in GLSL ES 3.00, used as local variable names. WebGL accepts
// some and refuses the rest; the engine must agree word by word.
const WORDS = [
  'input', 'output', 'filter', 'sample', 'buffer', 'shared', 'coherent', 'volatile', 'restrict', 'readonly', 'writeonly', 'precise',
  'subroutine', 'patch', 'resource', 'smooth', 'noperspective', 'atomic_uint', 'image2D', 'iimage2D', 'sampler1D', 'samplerBuffer',
  'sampler2DRect', 'sampler2DMS', 'dvec2', 'dmat2', 'double', 'half', 'fixed', 'long', 'short', 'unsigned', 'superp', 'common',
  'partition', 'active', 'goto', 'inline', 'noinline', 'public', 'static', 'extern', 'external', 'interface', 'class', 'union',
  'enum', 'typedef', 'template', 'this', 'packed', 'asm', 'sizeof', 'cast', 'namespace', 'using', 'hvec2', 'fvec2', 'texture',
  'noise', 'fma', 'frexp', 'ldexp', 'bitCount', 'findMSB', 'textureGather', 'barrier', 'imageLoad', 'texture2D', 'shadow2D',
  'mod', 'step', 'a__b', '_x', 'main2', 'gl_custom', 'webgl_x', '_webgl_x', 'ANGLE_x', '_ANGLE', 'pm_inverse', 'pm_modf',
  'attribute', 'varying', 'hvec3', 'dvec3', 'dvec4', 'fvec4', 'sampler3DRect', 'image1D', 'image3D', 'imageCube', 'uimage2D',
  'image1DArray', 'image2DArray', 'imageBuffer', 'sampler1DShadow', 'sampler1DArray', 'isampler1D', 'usampler1DArray',
  'sampler2DRectShadow', 'isampler2DRect', 'isamplerBuffer', 'isampler2DMS', 'sampler2DMSArray', 'dmat3', 'dmat2x3',
  'samplerCubeArray', 'image2DMS', 'sampler', 'samplerShadow', 'texture3D', 'textureCube', 'texture2DArray', 'itexture2D',
  'subpassInput', 'float16_t', 'int64_t', 'f16vec2', 'accelerationStructureEXT', 'rayQueryEXT', 'demote', 'nonuniformEXT',
  'lowp_x', 'highp', 'precision', 'invariant', 'flat', 'centroid', 'layout', 'uniform', 'switch', 'default', 'struct',
];
for (const w of WORDS) add('identifiers', `local-${w}`, `float ${w} = s * 2.0; o = show(${w});`);
add('identifiers', 'user-function-fma', `o = show(fma(s, 2.0, 1.0));`, { decls: 'float fma(float a, float b, float c){ return a * b + c; }\n' });
add('identifiers', 'user-function-noise', `o = show(noise(v2));`, { decls: 'float noise(vec2 x){ return fract(sin(dot(x, vec2(12.9898, 78.233))) * 43758.5453); }\n' });
add('identifiers', 'user-function-texture2D', `o = texture2D(u_tex, v_uv);`, { decls: 'vec4 texture2D(sampler2D s, vec2 uv){ return texture(s, uv); }\n' });
add('identifiers', 'define-texture2D-as-texture', `o = texture2D(u_tex, v_uv);`, { decls: '#define texture2D texture\n' });
add('identifiers', 'user-function-dFdxFine', `o = show(dFdxFine(s));`, { decls: 'float dFdxFine(float x){ return x * 3.0; }\n' });
add('identifiers', 'user-function-textureGather', `o = textureGather(u_tex, v_uv);`, { decls: 'vec4 textureGather(sampler2D s, vec2 uv){ return texture(s, uv).bgra; }\n' });
add('identifiers', 'struct-field-keywords', `S x; x.shared = s; x.buffer = s * 2.0; x.texture2D = 1.0; o = show(vec3(x.shared, x.buffer, x.texture2D));`, { decls: 'struct S { float shared; float buffer; float texture2D; };\n' });
add('identifiers', 'redefine-builtin-sin', `o = show(sin(s));`, { decls: 'float sin(float x){ return x; }\n' }); // ES 3.00 forbids redefining built-ins
add('identifiers', 'overload-builtin-sin', `o = show(sin(i));`, { decls: 'float sin(int x){ return float(x); }\n' });
add('identifiers', 'long-identifier-1024', `float ${'a'.repeat(1024)} = s; o = show(${'a'.repeat(1024)});`);

// Undefined-behavior probes: compile parity only matters; pixels may differ.
add('ub', 'int-division-by-zero', FI((x) => `${x} / (${x} - ${x})`));
add('ub', 'shift-by-32', `o = show(ivec2(i << (31 + int(v_uv.x * 2.0)), 1 >> 33));`);
add('ub', 'out-of-bounds-dynamic-array', `float a[4] = float[4](1.0, 2.0, 3.0, 4.0); o = show(a[int(v_uv.x * 8.0)]);`);

if (args.includes('--summarize')) {
  summarize();
} else {
  mkdirSync(OUT, { recursive: true });
  const base = JSON.parse(readFileSync(join(WORK, 'corpus.json'), 'utf8'));
  const calls = [
    { fn: '1i', name: 'u_tex', v: [0] },
    { fn: 'm3', name: 'u_m', v: [256, 0, 0, 0, 144, 0, 0, 0, 1] },
    { fn: '2f', name: 'u_res', v: [256, 144] },
    { fn: '4f', name: 'u_uv', v: [0, 0, 1, 1] },
    { fn: '2f', name: 'u_pxOrigin', v: [0, 0] },
  ];
  const ids = new Set<string>();
  const cases = probes.map((probe) => {
    const id = `coverage/${probe.group}/${probe.name}`;
    if (ids.has(id)) throw new Error(`duplicate probe ${id}`);
    ids.add(id);
    const frag = program(probe);
    return { id, group: probe.group, kind: 'probe', frag, lineOffset: 0, variants: [{ name: 'defaults', width: 256, height: 144, passes: [{ calls, units: [{ unit: 0, src: 'layer' }] }] }] };
  });
  const images = base.images.map((image: any) => ({ ...image, file: join('..', image.file) }));
  writeFileSync(join(OUT, 'corpus.json'), JSON.stringify({ vertex: base.vertex, images, cases }, null, 1));
  console.log(`${cases.length} probes in ${new Set(probes.map((p) => p.group)).size} groups → ${join(OUT, 'corpus.json')}`);
}

function summarize() {
  const corpus = JSON.parse(readFileSync(join(OUT, 'corpus.json'), 'utf8'));
  const webglDir = join(WORK, args.includes('--webgl') ? args[args.indexOf('--webgl') + 1]! : 'webgl/coverage-metal');
  const resultsDir = join(OUT, 'results');
  const status = JSON.parse(readFileSync(join(webglDir, 'status.json'), 'utf8'));
  const read = (name: string) => existsSync(join(resultsDir, `${name}.jsonl`)) ? readFileSync(join(resultsDir, `${name}.jsonl`), 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l)) : [];
  const paths: Record<string, any[]> = { naga: read('naga'), spvc: read('spvc') };
  const groups = [...new Set<string>(corpus.cases.map((c: any) => c.group))];
  // WebGL outcome per probe: refused at link, linked but the draw raised a GL
  // error (nothing renders in the compositor), or drawn.
  const webgl = (id: string) => {
    const s = status.cases[id];
    if (!s || !s.compiled) return 'refused';
    return s.glError ? `draw error ${s.glError}` : 'drawn';
  };
  const classify = (r: any) => {
    const w = webgl(r.id);
    if (w === 'refused') return r.compiled ? 'engine-only accept' : 'both refuse';
    if (w !== 'drawn') return r.compiled ? `WebGL ${w}; engine renders` : `WebGL ${w}; engine refuses`;
    if (!r.compiled) return 'WebGL draws; engine refuses';
    return r.variants.every((v: any) => v.metrics?.pass) ? 'match' : 'pixel mismatch';
  };
  const out: string[] = [];
  out.push(`Coverage sweep: ${corpus.cases.length} probes. WebGL reference: ${status.renderer} (Chromium ${status.browser}).`, '');
  for (const [name, rs] of Object.entries(paths)) {
    if (!rs.length) continue;
    out.push(`### ${name}`, '', '| Group | Probes | WebGL draws | Engine matches (of drawn) | WebGL refuses (engine agrees) | WebGL draw error (engine refuses) |', '|---|---|---|---|---|---|');
    const row = (label: string, g: any[]) => {
      const drawn = g.filter((r) => webgl(r.id) === 'drawn');
      const refused = g.filter((r) => webgl(r.id) === 'refused');
      const drawErr = g.filter((r) => webgl(r.id).startsWith('draw error'));
      out.push(`| ${label} | ${g.length} | ${drawn.length} | ${drawn.filter((r) => classify(r) === 'match').length}/${drawn.length} | ${refused.length} (${refused.filter((r) => !r.compiled).length}) | ${drawErr.length} (${drawErr.filter((r) => !r.compiled).length}) |`);
    };
    for (const g of groups) row(g, rs.filter((r) => r.group === g));
    row('**all**', rs);
    out.push('');
  }
  out.push('### Every probe where an engine path and WebGL disagree', '', '| Probe | WebGL | naga | SPIRV-Cross |', '|---|---|---|---|');
  const detail = (r: any) => {
    if (!r) return 'not run';
    const c = classify(r);
    if (c === 'match' || c === 'both refuse') return c;
    if (!r.compiled) return `${c}: ${r.failure?.stage}: ${(r.failure?.message ?? '').replace(/\s+/g, ' ').slice(0, 140)}`;
    const bad = r.variants.find((v: any) => !v.metrics?.pass);
    return bad ? `${c}: ${bad.metrics ? `max ${bad.metrics.max_diff_8bit} CV, ${bad.metrics.over_one} px over 1 CV` : bad.note}` : c;
  };
  for (const c of corpus.cases) {
    const n = paths.naga.find((r) => r.id === c.id), x = paths.spvc.find((r) => r.id === c.id);
    const dn = detail(n), dx = detail(x);
    if ([dn, dx].every((d) => d === 'match' || d === 'both refuse' || d === 'not run')) continue;
    const s = status.cases[c.id];
    const w = webgl(c.id) + (s && !s.compiled ? `: ${(s.log ?? '').replace(/\s+/g, ' ').slice(0, 100)}` : '');
    out.push(`| ${c.id.replace('coverage/', '')} | ${w.replace(/\|/g, '\\|')} | ${dn.replace(/\|/g, '\\|')} | ${dx.replace(/\|/g, '\\|')} |`);
  }
  console.log(out.join('\n'));
}
