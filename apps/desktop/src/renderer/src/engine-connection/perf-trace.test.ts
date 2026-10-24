import { expect, it, vi } from 'vitest';
import { RenderPerfTrace } from './perf-trace';

it('keeps the latest timings in order, drains them, and records nothing unless enabled', () => {
  const clock = vi.spyOn(performance, 'now').mockReturnValue(15);
  try {
    const trace = new RenderPerfTrace(2);
    trace.record('off', 1);
    expect(trace.take()).toEqual([]);
    trace.enabled = true;
    for (let revision = 1; revision <= 3; revision++) trace.record('base', revision, 42, 10, 4);
    expect(trace.take()).toEqual([2, 3].map(revision => ({ stage: 'base', revision, time: 42, at: 10, ms: 5, count: 4 })));
    expect(trace.take()).toEqual([]);
  } finally { clock.mockRestore(); }
});
