/*
 * Writes the resource-limit probes: programs on both sides of every limit
 * WebGL (ANGLE in Chromium) enforces or stumbles on, so the engine's limits
 * stage can be checked against WebGL's decisions with
 * `scripts/webgl-compile.ts` and `scripts/compile-parity.ts`.
 *
 * Usage: bun scripts/limit-probes.ts [--out work/fuzz/limits]
 */
import { mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const SPIKE = join(import.meta.dir, '..');
const args = process.argv.slice(2);
const dir = join(SPIKE, args.includes('--out') ? args[args.indexOf('--out') + 1]! : 'work/fuzz/limits');

const H = '#version 300 es\nprecision highp float;\nin vec2 v_uv; in vec2 v_st; in vec2 v_px;\nuniform sampler2D u_tex;\nuniform vec2 u_res;\nout vec4 o;\n';
const probes: [string, string][] = [];
const add = (name: string, body: string) => probes.push([name, H + body]);

// Private (function and global) variable size: ANGLE rejects a variable above 64 KiB.
for (const n of [16384, 16385, 65536, 262144]) add(`local-float-array-${n}`, `void main(){ float a[${n}]; a[int(u_res.x)] = 1.0; o = vec4(a[int(u_res.y)]); }\n`);
for (const n of [4096, 4097, 16384]) add(`global-vec4-array-${n}`, `vec4 g[${n}]; void main(){ g[int(u_res.x)] = vec4(1.0); o = g[int(u_res.y)]; }\n`);
for (const n of [1024, 1025]) add(`local-mat4-array-${n}`, `void main(){ mat4 a[${n}]; a[int(u_res.x)] = mat4(1.0); o = a[int(u_res.y)][0]; }\n`);
add('two-local-arrays-16000', `void main(){ float a[16000]; float b[16000]; a[int(u_res.x)] = 1.0; b[int(u_res.y)] = 2.0; o = vec4(a[int(u_res.y)] + b[int(u_res.x)]); }\n`);
add('const-array-100000', `const float k[100000] = float[100000](${Array(100000).fill('1.0').join(',')});\nvoid main(){ o = vec4(k[int(u_res.x)]); }\n`);
add('local-array-2^28', `void main(){ float a[268435456]; a[int(u_res.x)] = 1.0; o = vec4(a[0]); }\n`);

// Active uniform vectors: MAX_FRAGMENT_UNIFORM_VECTORS is 1024 here (u_res takes one).
for (const n of [1023, 1024]) add(`uniform-vec4-used-${n}`, `uniform vec4 big[${n}];\nvoid main(){ o = big[int(u_res.x)]; }\n`);
add('uniform-vec4-unused-5000', `uniform vec4 big[5000];\nvoid main(){ o = vec4(u_res, 0.0, 1.0); }\n`);
add('uniform-float-used-2000', `uniform float f[2000];\nvoid main(){ o = vec4(f[int(u_res.x)]); }\n`);
add('uniform-vec4-2^27', `uniform vec4 big[134217728];\nvoid main(){ o = big[int(u_res.x)]; }\n`);

// Call stack depth (256 including main) and recursion.
for (const d of [255, 256]) {
  let s = 'float g0(float x){ return x + 1.0; }\n';
  for (let i = 1; i < d; i++) s += `float g${i}(float x){ return g${i - 1}(x) * 0.5; }\n`;
  add(`call-depth-${d}`, `${s}void main(){ o = vec4(g${d - 1}(u_res.x)); }\n`);
}
for (const d of [20, 24, 2000]) {
  let s = 'float g0(float x){ return x + 1.0; }\n';
  for (let i = 1; i < d; i++) s += `float g${i}(float x){ return g${i - 1}(x) + g${i - 1}(x * 0.5); }\n`;
  add(`call-tree-same-callee-${d}`, `${s}void main(){ o = vec4(g${d - 1}(u_res.x)); }\n`);
}
for (const d of [20, 24, 40]) {
  let s = 'float g0(float x){ return x + 1.0; }\nfloat h0(float x){ return x * 2.0; }\n';
  for (let i = 1; i < d; i++) s += `float g${i}(float x){ return g${i - 1}(x) + h${i - 1}(x * 0.5); }\nfloat h${i}(float x){ return h${i - 1}(x) - g${i - 1}(x); }\n`;
  add(`call-dag-two-callees-${d}`, `${s}void main(){ o = vec4(g${d - 1}(u_res.x)); }\n`);
}
add('recursion', `float r(float x){ return x > 0.0 ? r(x - 1.0) : 0.0; }\nvoid main(){ o = vec4(r(u_res.x)); }\n`);
add('mutual-recursion', `float b(float x);\nfloat a(float x){ return x > 0.0 ? b(x - 1.0) : 0.0; }\nfloat b(float x){ return a(x * 0.5); }\nvoid main(){ o = vec4(a(u_res.x)); }\n`);
add('overload-calls-overload', `float h(float x){ return fract(sin(x) * 43758.5453); }\nfloat h(vec2 p){ return h(dot(p, vec2(12.9898, 78.233))); }\nvoid main(){ o = vec4(h(v_st)); }\n`);

// Struct nesting (4 levels).
for (const n of [4, 5]) {
  let s = 'struct S0 { float x; };\n';
  for (let i = 1; i < n; i++) s += `struct S${i} { S${i - 1} a; float b; };\n`;
  add(`struct-nesting-${n}`, `${s}void main(){ S${n - 1} s; o = vec4(1.0); }\n`);
}
{
  let s = 'struct S0 { float x; };\n';
  for (let i = 1; i < 500; i++) s += `struct S${i} { S${i - 1} a; S${i - 1} b; };\n`;
  add('struct-doubling-500', `${s}uniform S20 u_s;\nvoid main(){ o = vec4(1.0); }\n`);
}

// Expression complexity (AST depth 256), nesting, chains.
for (const d of [120, 126]) add(`expression-chain-${d}`, `void main(){ float f = u_res.x${' + u_res.y * (u_res.x'.repeat(d)}${')'.repeat(d)}; o = vec4(f); }\n`);
for (const n of [200, 230, 240, 245, 250, 252, 254, 1000, 100000]) add(`add-chain-${n}`, `void main(){ float f = u_res.x${' + u_res.y'.repeat(n)}; o = vec4(f); }\n`);
for (const n of [120, 200, 250]) add(`nested-mix-${n}`, `void main(){ float f = ${'mix(u_res.y, '.repeat(n)}u_res.x${', 0.5)'.repeat(n)}; o = vec4(f); }\n`);
for (const n of [120, 200, 250]) add(`nested-ternary-${n}`, `void main(){ float f = ${'u_res.x > 0.5 ? u_res.y : ('.repeat(n)}u_res.x${')'.repeat(n)}; o = vec4(f); }\n`);
for (const n of [1000, 100000]) add(`swizzle-chain-${n}`, `void main(){ vec4 v = vec4(u_res, 1.0, 1.0); o = v${'.wzyx'.repeat(n)}; }\n`);
for (const d of [1000, 20000]) add(`nested-parens-${d}`, `void main(){ float f = ${'('.repeat(d)}u_res.x${')'.repeat(d)}; o = vec4(f); }\n`);
add('nested-blocks-10000', `void main(){ ${'{'.repeat(10000)} o = vec4(1.0); ${'}'.repeat(10000)} }\n`);
add('nested-ifs-2000', `void main(){ o = vec4(0.0); ${'if (u_res.x > 0.0) {'.repeat(2000)} o = vec4(1.0); ${'}'.repeat(2000)} }\n`);
add('nested-loops-64', `void main(){ float a = 0.0; ${Array.from({ length: 64 }, (_, i) => `for (int i${i} = 0; i${i} < 4; i${i}++) `).join('')} a += 1.0; o = vec4(a); }\n`);

// Preprocessor expansion.
for (const n of [16, 18, 20, 22, 24]) {
  let m = '#define A0 1.0\n';
  for (let i = 1; i <= n; i++) m += `#define A${i} (A${i - 1} + A${i - 1})\n`;
  add(`macro-doubling-2^${n}`, `${m}void main(){ o = vec4(A${n}); }\n`);
}
{
  let m = '#define F0(x) (x)\n';
  for (let i = 1; i <= 24; i++) m += `#define F${i}(x) F${i - 1}(x) + F${i - 1}(x)\n`;
  add('function-macro-doubling-2^24', `${m}void main(){ o = vec4(F24(u_res.x)); }\n`);
}
add('macro-in-dead-branch', `#if 0\n${Array.from({ length: 30 }, (_, i) => (i === 0 ? '#define A0 1.0' : `#define A${i} (A${i - 1} + A${i - 1})`)).join('\n')}\nfloat x = A29;\n#endif\nvoid main(){ o = vec4(1.0); }\n`);

// Source size and identifiers.
for (const kb of [64, 256, 512, 1024, 4096]) {
  const line = 'f += sin(f) * 0.5;\n';
  add(`source-${kb}KB`, `void main(){ float f = u_res.x;\n${line.repeat(Math.floor((kb * 1024) / line.length))} o = vec4(f); }\n`);
}
add('comment-source-4096KB', `// ${'x'.repeat(4096 * 1024)}\nvoid main(){ o = vec4(1.0); }\n`);
for (const n of [1024, 1025]) add(`identifier-${n}`, `void main(){ float x${'a'.repeat(n - 1)} = 1.0; o = vec4(x${'a'.repeat(n - 1)}); }\n`);
{
  let s = '';
  for (let i = 0; i < 20000; i++) s += `float f${i}(float x){ return x * ${i}.0; }\n`;
  add('functions-20000', `${s}void main(){ o = vec4(f19999(1.0)); }\n`);
}

// Runtime-heavy programs WebGL accepts (render-time limits, not translation).
add('loop-1e9-texture', `void main(){ vec4 a = vec4(0.0); for (int i = 0; i < 1000000000; i++) a += texture(u_tex, v_st + float(i)); o = a; }\n`);
add('constant-loop-1e6', `void main(){ float a = 0.0; for (int i = 0; i < 1000000; i++) a += sin(float(i)); o = vec4(a); }\n`);

rmSync(dir, { recursive: true, force: true });
mkdirSync(dir, { recursive: true });
probes.forEach(([name, source], i) => {
  writeFileSync(join(dir, `${i}.frag`), source);
  writeFileSync(join(dir, `${i}.name`), name);
});
console.log(`${probes.length} probes in ${dir}`);
