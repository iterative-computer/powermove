import { describe, expect, it, vi } from 'vitest';
import { disposeModel, loadGltf } from './import-model';

const decoder = vi.hoisted(() => ({ loads: 0 }));
vi.mock('three/addons/loaders/GLTFLoader.js', async () => {
  decoder.loads++;
  return vi.importActual('three/addons/loaders/GLTFLoader.js');
});

function triangleGlb(): Blob {
  const json = JSON.stringify({
    asset: { version: '2.0' }, scene: 0,
    scenes: [{ nodes: [0] }], nodes: [{ mesh: 0 }],
    meshes: [{ primitives: [{ attributes: { POSITION: 0 } }] }],
    buffers: [{ byteLength: 36 }],
    bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: 36 }],
    accessors: [{ bufferView: 0, componentType: 5126, count: 3, type: 'VEC3', min: [-1, -1, 0], max: [1, 1, 0] }]
  });
  const text = new TextEncoder().encode(json.padEnd(Math.ceil(json.length / 4) * 4, ' '));
  const bytes = new Uint8Array(20 + text.length + 8 + 36), view = new DataView(bytes.buffer);
  view.setUint32(0, 0x46546c67, true); view.setUint32(4, 2, true); view.setUint32(8, bytes.length, true);
  view.setUint32(12, text.length, true); view.setUint32(16, 0x4e4f534a, true); bytes.set(text, 20);
  view.setUint32(20 + text.length, 36, true); view.setUint32(24 + text.length, 0x004e4942, true);
  bytes.set(new Uint8Array(new Float32Array([-1, -1, 0, 1, -1, 0, 0, 1, 0]).buffer), 28 + text.length);
  return new Blob([bytes], { type: 'model/gltf-binary' });
}

describe('on-demand model decoder', () => {
  it('keeps startup and rejected files free of decoder initialization', async () => {
    expect(decoder.loads).toBe(0);
    await expect(loadGltf(new Blob(['invalid JSON']))).rejects.toThrow();
    await expect(loadGltf(new Blob([JSON.stringify({ buffers: [{ uri: 'https://example.com/model.bin' }] })]))).rejects.toThrow('self-contained GLB');
    // CAD budgets: 10 million triangles in up to 65,536 parts.
    await expect(loadGltf(new Blob([JSON.stringify({ accessors: [{ count: 30_000_001 }] })]))).rejects.toThrow('oversized glTF');
    await expect(loadGltf(new Blob([JSON.stringify({ nodes: Array.from({ length: 65_537 }, () => ({})) })]))).rejects.toThrow('complexity budget');
    expect(decoder.loads).toBe(0);
  });

  it('decodes the first real model and shares the decoder on later imports', async () => {
    const [first, second] = await Promise.all([loadGltf(triangleGlb()), loadGltf(triangleGlb())]);
    expect(decoder.loads).toBe(1);
    for (const model of [first, second]) {
      expect(model.vertices).toBe(3); expect(model.triangles).toBe(1); expect(model.animations).toEqual([]);
      const geometry = model.object3d.children[0]!.children[0]!.children[0] as any;
      expect(Array.from(geometry.geometry.attributes.position.array)).toEqual([-1, -1, 0, 1, -1, 0, 0, 1, 0]);
      const disposed = vi.spyOn(geometry.geometry, 'dispose');
      disposeModel(model.object3d); expect(disposed).toHaveBeenCalledOnce();
    }
    expect(first.object3d).not.toBe(second.object3d);
  });
});
