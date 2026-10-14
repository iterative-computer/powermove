/**
 * Reference hashes of three.js r180 primitive geometry, built exactly as
 * `primitiveGeometry` (apps/desktop/src/renderer/src/core/scene3d/runtime.ts)
 * builds it, for tests/three_primitives.rs. Pure JavaScript (no WebGL).
 *
 *   bun engine/crates/pm-geom3d/tools/three-primitives.ts
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

const cases: Array<[string, Record<string, number>]> = [
  ['box', {}], ['box', { width: 1.4, height: 1.4, depth: 1.4 }], ['box', { width: 2.5, height: 0.3, depth: 7 }],
  ['sphere', {}], ['sphere', { radius: 1, segments: 48 }], ['sphere', { radius: 0.37, segments: 7 }], ['sphere', { radius: 3, segments: 128 }],
  ['plane', {}], ['plane', { width: 2.4, height: 1.6 }],
  ['cylinder', {}], ['cylinder', { radiusTop: 0.2, radiusBottom: 0.7, height: 2, segments: 17 }], ['cylinder', { radius: 1.1, height: 0.5, segments: 3 }],
  ['cylinder', { radiusTop: 0, radiusBottom: 0.5, height: 1 }],
  ['cone', {}], ['cone', { radius: 0.8, height: 1.7, segments: 9 }],
  ['torus', {}], ['torus', { radius: 1, tube: 0.4, segments: 32 }], ['torus', { radius: 0.6, tube: 0.18, segments: 5 }],
];
const hash = (a: ArrayLike<number> & { buffer: ArrayBufferLike; byteOffset: number; byteLength: number }) =>
  createHash('sha256').update(new Uint8Array(a.buffer, a.byteOffset, a.byteLength)).digest('hex');
const out = cases.map(([primitive, parameters]) => {
  const g = primitiveGeometry({ primitive, parameters });
  const index = Uint32Array.from(g.index.array as ArrayLike<number>);
  return {
    primitive, parameters,
    vertices: g.attributes.position.count,
    indices: index.length,
    position: hash(g.attributes.position.array), normal: hash(g.attributes.normal.array), uv: hash(g.attributes.uv.array),
    index: hash(index),
  };
});
const file = path.join(here, '../tests/fixtures/three-primitives.json');
writeFileSync(file, JSON.stringify({ three: version, cases: out }, null, 1) + '\n');
console.log(`${out.length} cases → ${path.relative(repo, file)}`);
