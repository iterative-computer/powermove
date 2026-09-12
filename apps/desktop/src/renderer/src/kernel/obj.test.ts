import { describe, expect, it } from 'vitest';
import { parseObj } from './obj';

describe('Wavefront OBJ parser', () => {
  it('triangulates polygons, resolves negative indices, and fills missing normals', () => {
    const mesh = parseObj(`
      v -1 -1 0
      v  1 -1 0
      v  1  1 0
      v -1  1 0
      vt 0 0
      vt 1 0
      vt 1 1
      vt 0 1
      f -4/1 -3/2 -2/3 -1/4
    `);
    expect(mesh.sourceVertexCount).toBe(4);
    expect(mesh.triangleCount).toBe(2);
    expect(mesh.vertexCount).toBe(6);
    expect([...mesh.normals]).toEqual(Array.from({ length: 6 }, () => [0, 0, 1]).flat());
    expect(Math.max(...mesh.positions)).toBeCloseTo(Math.SQRT1_2);
  });

  it('uses supplied normals and rejects invalid or empty geometry', () => {
    const mesh = parseObj('v 0 0 0\nv 1 0 0\nv 0 1 0\nvn 0 0 -2\nf 1//1 2//1 3//1');
    expect([...mesh.normals.slice(0, 3)]).toEqual([0, 0, -1]);
    expect(() => parseObj('v 0 0 0')).toThrow(/faces/);
    expect(() => parseObj('v 0 0 0\nv 1 0 0\nf 1 2 3')).toThrow(/out of range/);
  });
});
