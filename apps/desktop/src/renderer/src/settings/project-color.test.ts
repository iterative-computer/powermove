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
    expect(projectColorSettings(PM)).toEqual({ linearBlending: false });
    expect(applyProjectColor(PM, { linearBlending: true })).toBe(true);
    expect(PM.proj.color).toEqual({ mode: 'standard', linearBlending: true, blendDomain: 'hdrSafe' });
    expect(PM.hist.do).toHaveBeenCalledTimes(1);
    expect(PM.hist.do.mock.calls[0][0]).toBe('Blend in linear light');
    expect(PM.bus.emit).toHaveBeenCalledWith('project');
    expect(projectColorSettings(PM)).toEqual({ linearBlending: true });
    // No change, no history entry.
    expect(applyProjectColor(PM, { linearBlending: true })).toBe(false);
    expect(PM.hist.do).toHaveBeenCalledTimes(1);
    expect(applyProjectColor(PM, { linearBlending: false })).toBe(true);
    expect('color' in PM.proj).toBe(false);
    undo.pop()!();
    expect(projectColorSettings(PM)).toEqual({ linearBlending: true });
  });
});
