import { expect, it } from 'vitest';
import { buildBvh } from './bvh';

it('builds an empty tree with two absent children', () => {
  const tree = buildBvh(new Float32Array());
  expect(tree.order.length).toBe(0);
  expect(new Uint32Array(tree.nodes)[7]).toBe(0xffffffff);
  expect(new Uint32Array(tree.nodes)[15]).toBe(0xffffffff);
});

it('rejects incomplete triangles and non-finite positions before building', () => {
  expect(() => buildBvh(new Float32Array(8))).toThrow('triangles');
  expect(() => buildBvh(new Float32Array([NaN, 0, 0, 1, 0, 0, 0, 1, 0]))).toThrow('finite');
});

it('bounds skewed tree depth to fit the shared shader traversal stacks', () => {
  const corners = new Float32Array(240 * 9);
  for (let i = 0; i < 240; i++) {
    const x = 2 ** (i / 2);
    corners.set([x, 0, 0, x, 1, 0, x, 0, 1], i * 9);
  }
  const tree = buildBvh(corners), nodes = new Uint32Array(tree.nodes);
  let leaves = 0, depth = 0;
  const stack = [[0, 0]];
  while (stack.length) {
    const [index, level] = stack.pop()!;
    depth = Math.max(depth, level!);
    for (const side of [0, 8]) {
      const offset = index! * 16 + side, count = nodes[offset + 7]!;
      if (count === 0xffffffff) continue;
      if (!count) stack.push([nodes[offset + 3]!, level! + 1]);
      else leaves += count;
    }
  }
  expect(depth).toBeLessThanOrEqual(20);
  expect(leaves).toBe(240);
});
