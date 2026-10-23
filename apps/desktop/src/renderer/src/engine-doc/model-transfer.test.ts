import { expect, it } from 'vitest';
import { decodeModelPacket } from './model-transfer';

function packet(scene: any, scalars: number[], integers: number[] = []): Uint8Array {
  const header = new TextEncoder().encode(JSON.stringify(scene));
  const start = Math.ceil((4 + header.length) / 4) * 4;
  const bytes = new Uint8Array(start + (scalars.length + integers.length) * 4);
  new DataView(bytes.buffer).setUint32(0, header.length, true);
  bytes.set(header, 4);
  const payload = new DataView(bytes.buffer, start);
  scalars.forEach((value, index) => payload.setFloat32(index * 4, value, true));
  integers.forEach((value, index) => payload.setUint32((scalars.length + index) * 4, value, true));
  return bytes;
}

it('decodes only explicit mesh fields as shared typed views', () => {
  const packed = (offset: number, length: number, components: number, type: 'f32' | 'u32' = 'f32') =>
    ({ packed: { offset, length, components, type } });
  const scene = { meshes: [{ primitives: [{
    positions: packed(0, 9, 3), normals: null, uv: packed(36, 2, 2),
    uv1: null, uv2: null, uv3: null, colors: null, tangents: null,
    joints: packed(56, 4, 4, 'u32'), weights: null, indices: packed(72, 3, 1, 'u32'),
    targets: [{ position: packed(44, 3, 3), normal: null }],
    extras: { foreign: packed(0, 9, 3) },
  }] }] };
  const bytes = packet(scene, [0, 0, 0, 1, 0, 0, 0, 1, 0, .25, .75, 0, 0, 1], [1, 2, 65535, 4, 0, 1, 2]);
  const decoded = decodeModelPacket(bytes);
  const primitive = decoded.meshes[0].primitives[0];
  expect(primitive.positions).toBeInstanceOf(Float32Array);
  expect(Array.from(primitive.positions)).toEqual([0, 0, 0, 1, 0, 0, 0, 1, 0]);
  expect(Array.from(primitive.uv)).toEqual([.25, .75]);
  expect(Array.from(primitive.targets[0].position)).toEqual([0, 0, 1]);
  expect(Array.from(primitive.joints)).toEqual([1, 2, 65535, 4]);
  expect(Array.from(primitive.indices)).toEqual([0, 1, 2]);
  expect(primitive.positions.buffer).toBe(bytes.buffer);
  expect(primitive.uv.buffer).toBe(bytes.buffer);
  expect(primitive.targets[0].position.buffer).toBe(bytes.buffer);
  expect(primitive.indices.buffer).toBe(bytes.buffer);
  expect(primitive.extras.foreign).toEqual(scene.meshes[0]!.primitives[0]!.extras.foreign);
});

it('rejects a marker outside the payload', () => {
  const bytes = packet({ meshes: [{ primitives: [{ positions: {
    packed: { offset: 32, length: 3, components: 3, type: 'f32' },
  } }] }] }, []);
  expect(() => decodeModelPacket(bytes)).toThrow('Invalid packed model attribute');
});
