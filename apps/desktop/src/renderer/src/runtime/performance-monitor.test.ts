import { expect, it } from 'vitest';
import { PerformanceMonitor } from './performance-monitor';
it('requires sustained cost, reports severe stalls immediately, and expires recovered work', () => {
  const monitor = new PerformanceMonitor();
  const info = { id: 'shader:1', name: 'Smoke', kind: 'shader' as const };
  monitor.record(info, 12, 0); monitor.record(info, 12, 1);
  expect(monitor.issues(1)).toEqual([]);
  monitor.record(info, 12, 2);
  expect(monitor.issues(2)[0]?.name).toBe('Smoke');
  for (let i = 0; i < 10; i++) monitor.record(info, 1, 3 + i);
  expect(monitor.issues(15)).toEqual([]);
  monitor.record({ ...info, id: 'extension:x' }, 60, 20);
  expect(monitor.issues(20)).toHaveLength(1);
  expect(monitor.issues(16_000)).toEqual([]);
});
