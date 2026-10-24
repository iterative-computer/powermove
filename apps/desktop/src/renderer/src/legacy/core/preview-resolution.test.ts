import { describe, expect, it, vi } from 'vitest';
import { installPreviewResolution } from './preview-resolution';

describe('preview resolution preferences', () => {
  it('restores the requested mode on restart independently of the adaptive scale', () => {
    const saved = new Map<string, unknown>();
    const make = () => {
      const PM: any = { quality: 1, perf: { auto: true }, bus: { emit: vi.fn() }, invalidate: vi.fn(),
        store: { get: (key: string, fallback: unknown) => saved.get(key) ?? fallback, set: (key: string, value: unknown) => saved.set(key, value) } };
      installPreviewResolution(PM); return PM;
    };
    const first = make(); first.setPreviewResolution('0.25');
    const second = make();
    expect(second).toMatchObject({ quality: .25, perf: { auto: false }, previewResolution: '0.25' });
    second.setPreviewResolution('auto'); second.quality = .25;
    expect(make()).toMatchObject({ quality: 1, perf: { auto: true }, previewResolution: 'auto' });
    expect(second.bus.emit).toHaveBeenCalledWith('quality');
  });

  it('ignores invalid saved modes and invalid requests without changing the viewer', () => {
    const PM: any = { quality: 1, perf: { auto: true }, bus: { emit: vi.fn() }, invalidate: vi.fn(),
      store: { get: () => 'corrupt', set: vi.fn() } };
    installPreviewResolution(PM);
    for (const value of [null, undefined, 0, '0.75', NaN, 'Quarter']) PM.setPreviewResolution(value);
    expect(PM.quality).toBe(1); expect(PM.previewResolution).toBeUndefined();
    expect(PM.bus.emit).not.toHaveBeenCalled(); expect(PM.store.set).not.toHaveBeenCalled();
  });
});
