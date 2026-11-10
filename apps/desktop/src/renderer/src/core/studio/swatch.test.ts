// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
const gpu = vi.hoisted(() => ({ ready: true, readBeauty: vi.fn(() => Promise.resolve(new Float32Array(96 * 96 * 4))), scenes: vi.fn(), extracts: vi.fn() }));
vi.mock('./gpu', () => ({ studioGpuReady: () => gpu.ready, initStudioGpu: async () => true, StudioTarget: class {
  setScene(...args: unknown[]) { gpu.scenes(...args); } setView() {} trace() {} resolve() {}
  readBeauty() { return gpu.readBeauty(); }
} }));
vi.mock('./extract', () => ({ MAP_SIZE: 1, extractScene: (scene: unknown) => { gpu.extracts(scene); return {}; } }));
import { studioSwatch } from './swatch';
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it('queues palette work across idle turns, shares sphere geometry and waits for each GPU completion', async () => {
  vi.useFakeTimers(); vi.stubGlobal('requestIdleCallback', undefined);
  const putImageData = vi.fn();
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ putImageData } as any);
  vi.stubGlobal('ImageData', class { constructor(readonly data: Uint8ClampedArray, readonly width: number, readonly height: number) {} });
  vi.spyOn(HTMLCanvasElement.prototype, 'toDataURL').mockReturnValue('data:image/png;swatch');
  let complete!: (pixels: Float32Array<ArrayBuffer>) => void;
  gpu.readBeauty.mockImplementationOnce(() => new Promise<Float32Array<ArrayBuffer>>(resolve => { complete = resolve; }));
  const first = studioSwatch('first-regression', { color: '#ff0000' });
  const second = studioSwatch('second-regression', { color: '#0000ff' });
  expect(gpu.extracts).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(0);
  expect(gpu.extracts).toHaveBeenCalledOnce();
  const pixels = new Float32Array(96 * 96 * 4); pixels.set([.25, .1, 0, .5]);
  complete(pixels); await first; await vi.advanceTimersByTimeAsync(1); await second;
  expect([...putImageData.mock.calls[0]![0].data.slice(0, 4)]).toEqual([128, 51, 0, 128]);
  expect(gpu.extracts).toHaveBeenCalledTimes(2);
  const [a, b] = gpu.extracts.mock.calls.map(([scene]) => scene.children[0]);
  expect(a.geometry).toBe(b.geometry); expect(a.material).not.toBe(b.material);
  await studioSwatch('first-regression', { color: '#ff0000' });
  expect(gpu.extracts).toHaveBeenCalledTimes(2);
});
