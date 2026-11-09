import { beforeEach, expect, it, vi } from 'vitest';
import * as THREE from 'three';
const gpu = vi.hoisted(() => ({ generation: 1, ready: true, allocations: 0, destroyed: 0, init: vi.fn() }));
vi.mock('./gpu', () => ({
  initStudioGpu: async () => { gpu.init(); gpu.ready = true; gpu.generation++; return {}; },
  studioGpuReady: () => gpu.ready,
  studioGpuGeneration: () => gpu.generation,
  studioMaxBindingBytes: () => 1024 * 1024 * 1024,
  studioGpuFailure: () => gpu.ready ? null : 'WebGPU device lost',
  StudioTarget: class {
    samples = 0;
    constructor() { gpu.allocations++; }
    destroy() { gpu.destroyed++; }
    setScene() {} setView() {} trace(n: number) { this.samples += n; } resolve() {} present() { return {}; }
  }
}));
import { WebGpuStudioBackend } from './webgpu-backend';
import { extractScene } from './extract';

beforeEach(() => { gpu.allocations = 0; gpu.destroyed = 0; gpu.generation = 1; gpu.ready = true; gpu.init.mockClear(); });
function backend() {
  const backend = new WebGpuStudioBackend(), graph = new THREE.Scene();
  const mesh = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshStandardMaterial()); mesh.userData.sceneId = 'box'; graph.add(mesh);
  backend.setPreparedScene(extractScene(graph, () => true, { sky: new THREE.Color(), ground: new THREE.Color(), intensity: 0 }), [], () => { throw new Error('Binary path expected'); });
  backend.setView({ camera: { position: [0, 0, 6], forward: [0, 0, -1], right: [1, 0, 0], up: [0, 1, 0], tanY: .5, aspect: 1, lensRadius: 0, focus: 6 }, exposure: 1, backdrop: [0, 0, 0], backdropOpaque: false, shadowCatcher: false, shadows: true, floorY: 0, environment: { zenith: [0, 0, 0], horizon: [0, 0, 0], ground: [0, 0, 0] } });
  return backend;
}
const request = (tier: 'live' | 'refine' | 'final' = 'live', width = 64, height = 64) => ({ tier, size: { width, height }, samples: 1 });

it('rejects an oversized output before allocating and keeps the valid scene usable', () => {
  const b = backend(); b.render(request());
  expect(() => b.render(request('final', 8192, 8192))).toThrow(/budget|dimensions/);
  expect(gpu.allocations).toBe(1);
  expect(b.render(request())).toMatchObject({ width: 64, height: 64 });
  b.dispose(); expect(gpu.destroyed).toBe(1);
});

it('recreates targets and replays the scene after browser device loss', async () => {
  const b = backend(); b.render(request());
  gpu.ready = false;
  await expect(Promise.resolve().then(() => b.render(request()))).resolves.toMatchObject({ width: 64, height: 64 });
  expect(gpu.init).toHaveBeenCalledOnce(); expect(gpu.destroyed).toBe(1); expect(gpu.allocations).toBe(2);
  b.dispose();
});

it('evicts inactive quality targets before exceeding the session allowance', () => {
  const b = backend();
  for (const tier of ['live', 'refine', 'final'] as const) b.render(request(tier, 2400, 1600));
  expect(gpu.destroyed).toBeGreaterThan(0);
  b.dispose(); expect(gpu.destroyed).toBe(gpu.allocations);
});
