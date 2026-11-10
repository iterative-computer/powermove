// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
const gpu = vi.hoisted(() => ({ ready: true, done: vi.fn(() => Promise.resolve()), scenes: vi.fn(), extracts: vi.fn() }));
vi.mock('./gpu', () => ({ studioGpuReady: () => gpu.ready, initStudioGpu: async () => true, StudioTarget: class {
  setScene(...args: unknown[]) { gpu.scenes(...args); } setView() {} trace() {} resolve() {}
  present() { return document.createElement('canvas'); } done() { return gpu.done(); }
} }));
vi.mock('./extract', () => ({ MAP_SIZE: 1, extractScene: (scene: unknown) => { gpu.extracts(scene); return {}; } }));
import { studioSwatch } from './swatch';
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it('queues palette work across idle turns, shares sphere geometry and waits for each GPU completion', async () => {
  vi.useFakeTimers(); vi.stubGlobal('requestIdleCallback', undefined);
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ drawImage() {} } as any);
  vi.spyOn(HTMLCanvasElement.prototype, 'toDataURL').mockReturnValue('data:image/png;swatch');
  let complete!: () => void;
  gpu.done.mockImplementationOnce(() => new Promise<void>(resolve => { complete = resolve; }));
  const first = studioSwatch('first-regression', { color: '#ff0000' });
  const second = studioSwatch('second-regression', { color: '#0000ff' });
  expect(gpu.extracts).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(0);
  expect(gpu.extracts).toHaveBeenCalledOnce();
  complete(); await first; await vi.advanceTimersByTimeAsync(1); await second;
  expect(gpu.extracts).toHaveBeenCalledTimes(2);
  const [a, b] = gpu.extracts.mock.calls.map(([scene]) => scene.children[0]);
  expect(a.geometry).toBe(b.geometry); expect(a.material).not.toBe(b.material);
  await studioSwatch('first-regression', { color: '#ff0000' });
  expect(gpu.extracts).toHaveBeenCalledTimes(2);
});
