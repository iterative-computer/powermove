/**
 * Reference hashes of the geometry three.js r180 builds for every scene
 * source kind (primitives with their groups, capsule, icosahedron, lathe,
 * extrude with and without bevel, mesh sources with and without normals,
 * index, and groups), exactly as `primitiveGeometry`
 * (apps/desktop/src/renderer/src/core/scene3d/runtime.ts) builds them, and of
 * the kernel's OBJ parser (`kernel/obj.ts`), for tests/three_geometry.rs.
 * Pure JavaScript (no WebGL). The OBJ parser runs in Node (V8, as in the
 * app): Bun's JavaScriptCore rounds `Math.hypot` differently.
 *
 *   bun engine/crates/pm-geom3d/tools/three-geometry.ts
 */
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const here = path.dirname(new URL(import.meta.url).pathname);
const repo = path.join(here, '../../../..');
const three = path.join(repo, 'apps/desktop/node_modules/three');
const version = JSON.parse(readFileSync(path.join(three, 'package.json'), 'utf8')).version;
if (version !== '0.180.0') throw new Error(`three.js ${version} is installed; the engine ports r180`);
const { primitiveGeometry } = await import(path.join(repo, 'apps/desktop/src/renderer/src/core/scene3d/runtime.ts'));
import { execFileSync } from 'node:child_process';

// A deterministic pseudo-random sequence for generated meshes and OBJ files.
let seed = 0x2545f491;
const rand = () => { seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0; return seed / 4294967296; };
// `+ 0` turns −0 into 0: JSON cannot store −0, and the stored source must
// rebuild the same geometry.
const round = (v: number, d = 4) => Number(v.toFixed(d)) + 0;

const star = (n: number, r0: number, r1: number) => Array.from({ length: n * 2 }, (_, i) => {
  const a = (i / (n * 2)) * Math.PI * 2, r = i % 2 ? r1 : r0;
  return [round(Math.cos(a) * r), round(Math.sin(a) * r)];
});
const blob = (n: number) => Array.from({ length: n }, (_, i) => {
  const a = (i / n) * Math.PI * 2, r = 0.6 + 0.3 * Math.sin(a * 3) + 0.1 * rand();
  return [round(Math.cos(a) * r), round(Math.sin(a) * r)];
});
const grid = (cols: number, rows: number, indexed: boolean, normals: boolean, uvs: boolean, groups: boolean) => {
  const positions: number[] = [], index: number[] = [], normal: number[] = [], uv: number[] = [];
  for (let y = 0; y <= rows; y++) for (let x = 0; x <= cols; x++) {
    positions.push(round(x / cols - 0.5), round(y / rows - 0.5), round(0.2 * Math.sin(x + y) + 0.05 * rand()));
    normal.push(0, 0, 1); uv.push(round(x / cols), round(y / rows));
  }
  for (let y = 0; y < rows; y++) for (let x = 0; x < cols; x++) {
    const a = y * (cols + 1) + x, b = a + 1, c = a + cols + 1, d = c + 1;
    index.push(a, b, d, a, d, c);
  }
  if (indexed) {
    const mesh: any = { positions, indices: index };
    if (normals) mesh.normals = normal;
    if (uvs) mesh.uvs = uv;
    if (groups) mesh.groups = [{ start: 0, count: index.length / 2 - (index.length / 2) % 3, materialIndex: 0 }, { start: index.length / 2 - (index.length / 2) % 3, count: index.length - (index.length / 2 - (index.length / 2) % 3), materialIndex: 1 }];
    return { mesh };
  }
  const flat: number[] = [], flatUv: number[] = [];
  for (const i of index) { flat.push(positions[i * 3]!, positions[i * 3 + 1]!, positions[i * 3 + 2]!); flatUv.push(uv[i * 2]!, uv[i * 2 + 1]!); }
  const mesh: any = { positions: flat };
  if (uvs) mesh.uvs = flatUv;
  return { mesh };
};

const sources: any[] = [
  ...[['box', {}], ['box', { width: 2.5, height: 0.3, depth: 7 }], ['sphere', { radius: 0.37, segments: 7 }],
    ['plane', { width: 2.4, height: 1.6 }], ['cylinder', { radiusTop: 0.2, radiusBottom: 0.7, height: 2, segments: 17 }],
    ['cylinder', { radiusTop: 0, radiusBottom: 0.5, height: 1 }], ['cylinder', { radiusTop: 0.5, radiusBottom: 0, height: 1, segments: 9 }],
    ['cone', { radius: 0.8, height: 1.7, segments: 9 }], ['torus', { radius: 0.6, tube: 0.18, segments: 5 }],
    ['capsule', {}], ['capsule', { radius: 0.5, height: 1.2, segments: 13 }], ['capsule', { radius: 0.25, height: 0.0001, segments: 3 }],
    ['icosahedron', {}], ['icosahedron', { radius: 1, detail: 0 }], ['icosahedron', { radius: 0.7, detail: 1 }],
    ['icosahedron', { radius: 1.3, detail: 3 }], ['icosahedron', { radius: 0.5, detail: 5 }]]
    .map(([primitive, parameters]) => ({ primitive, parameters })),
  { lathe: [[0, -0.5], [0.5, 0], [0, 0.5]] },
  { lathe: [[0, -0.8], [0.35, -0.75], [0.45, -0.2], [0.2, 0.3], [0.32, 0.7], [0, 0.8]], segments: 31 },
  { lathe: [[0.4, -0.5], [0.4, 0.5]], segments: 3 },
  { lathe: Array.from({ length: 40 }, (_, i) => [round(0.3 + 0.2 * Math.sin(i / 4)), round(i / 39 - 0.5)]), segments: 64 },
  { extrude: [[0, 0], [1, 0], [1, 1], [0, 1]], depth: 0.5 },
  { extrude: [[0, 0], [0, 1], [1, 1], [1, 0]], depth: 0.3, bevel: 0.05 },
  { extrude: star(5, 1, 0.45), depth: 0.4 },
  { extrude: star(5, 1, 0.45), depth: 0.4, bevel: 0.08 },
  { extrude: [[0, 0], [2, 0], [2, 0.4], [0.4, 0.4], [0.4, 2], [0, 2]], depth: 1, bevel: 0.1 },
  { extrude: [[0, 0], [1, 0], [1, 0], [1, 1], [0, 1], [0, 0]], depth: 0.2 },
  { extrude: blob(120), depth: 0.25 },
  { extrude: blob(90), depth: 0.25, bevel: 0.03 },
  grid(4, 3, true, true, true, false),
  grid(5, 4, true, false, true, true),
  grid(3, 3, true, false, false, false),
  grid(4, 2, false, false, true, false),
  grid(6, 5, false, false, false, false),
];

const hash = (a: ArrayLike<number> & { buffer: ArrayBufferLike; byteOffset: number; byteLength: number }) =>
  createHash('sha256').update(new Uint8Array(a.buffer, a.byteOffset, a.byteLength)).digest('hex');
const geometry = sources.map(source => {
  const g = primitiveGeometry(source);
  return {
    source,
    vertices: g.attributes.position.count,
    indexed: g.index !== null,
    indices: g.index ? g.index.count : g.attributes.position.count,
    position: hash(g.attributes.position.array), normal: hash(g.attributes.normal.array),
    uv: g.attributes.uv ? hash(g.attributes.uv.array) : null,
    index: g.index ? hash(Uint32Array.from(g.index.array as ArrayLike<number>)) : null,
    groups: g.groups.map((x: any) => [x.start, x.count, x.materialIndex ?? 0]),
  };
});

const objText = (faces: number, quads: boolean, withNormals: boolean, withUv: boolean, negative: boolean) => {
  const lines = ['# generated', 'o thing'];
  const n = faces + 3;
  for (let i = 0; i < n; i++) lines.push(`v ${(rand() * 4 - 2).toFixed(6)} ${(rand() * 3 - 1).toFixed(5)}  ${(rand() * 2).toExponential(3)}`);
  if (withNormals) for (let i = 0; i < n; i++) lines.push(`vn ${(rand() - 0.5).toFixed(4)} ${(rand() - 0.5).toFixed(4)} ${(rand() - 0.5).toFixed(4)}`);
  if (withUv) for (let i = 0; i < n; i++) lines.push(`vt ${rand().toFixed(4)} ${rand().toFixed(4)}`);
  lines.push('usemtl skin', 's 1');
  for (let f = 0; f < faces; f++) {
    const count = quads ? 4 : 3, refs: string[] = [];
    for (let k = 0; k < count; k++) {
      const v = (f + k) % n + 1, ref = negative ? String(v - n - 1) : String(v);
      refs.push(withNormals && withUv ? `${ref}/${ref}/${ref}` : withNormals ? `${ref}//${ref}` : withUv ? `${ref}/${ref}` : ref);
    }
    lines.push(`f ${refs.join(' ')}${f % 7 === 0 ? ' # face' : ''}`);
  }
  return lines.join(negative ? '\r\n' : '\n');
};
const objs = [
  objText(12, false, false, false, false), objText(30, true, true, true, false), objText(25, true, true, false, true),
  objText(40, false, false, true, false),
  'v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0.5 0.5 1\nf 1 2 3 4\nf 1 2 5\nf 2 3 5\nf 3 4 5\nf 4 1 5\n',
];
const v8 = `
  const { createHash } = await import('node:crypto');
  const { parseObj } = await import(${JSON.stringify(path.join(repo, 'apps/desktop/src/renderer/src/kernel/obj.ts'))});
  const hash = a => createHash('sha256').update(new Uint8Array(a.buffer, a.byteOffset, a.byteLength)).digest('hex');
  const texts = JSON.parse(require('node:fs').readFileSync(0, 'utf8'));
  console.log(JSON.stringify(texts.map(text => { const m = parseObj(text);
    return { text, vertices: m.vertexCount, position: hash(m.positions), normal: hash(m.normals), texcoord: hash(m.texcoords),
      center: m.bounds.center, radius: m.bounds.radius }; })));`;
const obj = JSON.parse(execFileSync('node', ['--no-warnings', '--input-type=commonjs', '-e', `(async () => {${v8}})()`], { input: JSON.stringify(objs) }).toString());
const file = path.join(here, '../tests/fixtures/three-geometry.json');
writeFileSync(file, JSON.stringify({ three: version, geometry, obj }, null, 1) + '\n');
console.log(`${geometry.length} sources, ${obj.length} OBJ files → ${path.relative(repo, file)}`);
