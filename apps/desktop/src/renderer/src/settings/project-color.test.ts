import { describe, expect, it, vi } from 'vitest';

import { applyProjectColor, projectColorSettings } from './project-color';

function registry(project: any) {
  const undo: Array<() => void> = [];
  const PM: any = {
    proj: project,
    hist: {
      do: vi.fn((_label: string, fn: () => void) => {
        const before = structuredClone(PM.proj.color);
        fn();
        undo.push(() => { if (before === undefined) delete PM.proj.color; else PM.proj.color = before; });
      })
    },
    touch: vi.fn(),
    bus: { emit: vi.fn() },
    invalidate: vi.fn(),
    autosave: vi.fn()
  };
  return { PM, undo };
}

describe('Settings › Project › Color writes', () => {
  it('makes each change one Undo step and removes the field at the default', () => {
    const { PM, undo } = registry({ id: 'p', layers: [] });
    expect(projectColorSettings(PM)).toEqual({ linearBlending: false, precision: 'float16' });
    expect(applyProjectColor(PM, { linearBlending: true })).toBe(true);
    expect(PM.proj.color).toEqual({ mode: 'standard', linearBlending: true, precision: 'float16', blendDomain: 'hdrSafe' });
    expect(PM.hist.do).toHaveBeenCalledTimes(1);
    expect(PM.hist.do.mock.calls[0][0]).toBe('Blend in linear light');
    expect(PM.bus.emit).toHaveBeenCalledWith('project');
    expect(applyProjectColor(PM, { precision: 'float32' })).toBe(true);
    expect(PM.hist.do.mock.calls[1][0]).toBe('Precision');
    expect(projectColorSettings(PM)).toEqual({ linearBlending: true, precision: 'float32' });
    // No change, no history entry.
    expect(applyProjectColor(PM, { precision: 'float32' })).toBe(false);
    expect(PM.hist.do).toHaveBeenCalledTimes(2);
    undo.pop()!();
    expect(projectColorSettings(PM)).toEqual({ linearBlending: true, precision: 'float16' });
    expect(applyProjectColor(PM, { linearBlending: false })).toBe(true);
    expect('color' in PM.proj).toBe(false);
  });
});
