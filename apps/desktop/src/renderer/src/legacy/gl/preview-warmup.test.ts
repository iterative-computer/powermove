import { describe, expect, it, vi } from 'vitest';
import { createPreviewWarmup } from './preview-warmup';

function fixture() {
  const project = { layers: Array.from({ length: 20 }, (_, i) => ({ id: String(i), type: 'shape', from: 4, dur: 5, on: true })) };
  const state = { key: '1', project, time: 2.5, blocked: false };
  let time = 0;
  const callbacks: Array<(deadline: { timeRemaining(): number }) => void> = [];
  const prepare = vi.fn(() => { time++; });
  const request = createPreviewWarmup(() => state, prepare, callback => callbacks.push(callback), () => time);
  return { state, callbacks, prepare, request, run: () => callbacks.shift()!({ timeRemaining: () => 10 }) };
}

describe('preview source warmup', () => {
  it('splits a simultaneous entrance into bounded idle slices without preparing current layers', () => {
    const f = fixture();
    f.state.project.layers.push({ id: 'current', type: 'shape', from: 0, dur: 8, on: true });
    f.request(); f.request(); expect(f.callbacks).toHaveLength(1);
    f.run(); expect(f.prepare).toHaveBeenCalledTimes(2);
    while (f.callbacks.length) f.run();
    expect(f.prepare).toHaveBeenCalledTimes(20);
    expect(f.prepare.mock.calls.every(([layer, time]: any) => layer.id !== 'current' && time === 4)).toBe(true);
    f.request(); expect(f.callbacks).toHaveLength(0);
  });

  it.each(['export', 'edit', 'project'] as const)('discards a queued slice after %s changes ownership', mode => {
    const f = fixture(); f.request();
    if (mode === 'export') f.state.blocked = true;
    if (mode === 'edit') f.state.key = '2';
    if (mode === 'project') f.state.project = { layers: [] };
    f.run(); expect(f.prepare).not.toHaveBeenCalled();
  });

  it('does not force work into an exhausted idle deadline', () => {
    const f = fixture(); f.request();
    f.callbacks.shift()!({ timeRemaining: () => 0 });
    expect(f.prepare).not.toHaveBeenCalled();
    expect(f.callbacks).toHaveLength(1);
  });

  it('lets the normal render own layers reached before an idle callback runs', () => {
    const f = fixture(); f.request(); f.state.time = 4; f.run();
    expect(f.prepare).not.toHaveBeenCalled();
  });
});
