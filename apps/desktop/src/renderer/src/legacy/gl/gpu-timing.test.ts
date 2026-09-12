import { expect, it, vi } from 'vitest';
import { GPUTiming } from './gpu-timing';
import { performanceMonitor } from '../../runtime/performance-monitor';
it('polls asynchronously, discards disjoint measurements and never waits for the GPU', () => {
  performanceMonitor.clear();
  let available = false, disjoint = false;
  const gl: any = { QUERY_RESULT_AVAILABLE: 1, QUERY_RESULT: 2, getExtension: () => ({ TIME_ELAPSED_EXT: 3, GPU_DISJOINT_EXT: 4 }),
    createQuery: () => ({}), beginQuery: vi.fn(), endQuery: vi.fn(), deleteQuery: vi.fn(),
    getParameter: () => disjoint, getQueryParameter: (_: any, kind: number) => kind === 1 ? available : 60_000_000 };
  const timer = new GPUTiming(gl), draw = vi.fn();
  timer.measure({ id: 'shader:a', name: 'Slow shader', kind: 'shader' }, draw); timer.poll();
  expect(draw).toHaveBeenCalledOnce(); expect(gl.deleteQuery).not.toHaveBeenCalled();
  available = true; timer.poll(); expect(performanceMonitor.issues()[0]?.ms).toBe(60);
  timer.measure({ id: 'shader:b', name: 'Invalid timing', kind: 'shader' }, draw);
  disjoint = true; timer.poll(); expect(performanceMonitor.issues()).toHaveLength(1);
  expect(gl.deleteQuery).toHaveBeenCalledTimes(2);
});
