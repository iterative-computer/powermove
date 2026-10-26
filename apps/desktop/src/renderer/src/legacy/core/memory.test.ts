import { afterEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './memory';

const originalWindow = (globalThis as any).window;
afterEach(() => {
  if (originalWindow === undefined) delete (globalThis as any).window;
  else (globalThis as any).window = originalWindow;
});

describe('memory budget manager', () => {
  it.each([0, 128, 256, 512, 1024, 2048])('restores a saved preview limit of %s MB', limit => {
    (globalThis as any).window = { addEventListener: vi.fn() };
    const PM = { store: { get: () => limit } } as PMRegistry;
    install(PM);
    expect(PM.Memory.budget('preview')).toBe(limit * 1024 * 1024);
  });

  it('shares the preview cap, releases frames immediately on lowering it, and supports Off', () => {
    (globalThis as any).window = { addEventListener: vi.fn() };
    const PM = {} as PMRegistry;
    install(PM);
    let preparedBytes = 2 * 1024 * 1024;
    let automaticBytes = 2 * 1024 * 1024;
    PM.GL = { previewFrames: { get bytes() { return automaticBytes; }, count: 2,
      trim: (target: number) => { automaticBytes = Math.min(automaticBytes, target); } } };
    PM.Preview = { get bytes() { return preparedBytes; }, count: 2, clear: () => { preparedBytes = 0; } };
    expect(PM.Memory.stats().preview.bytes).toBe(4 * 1024 * 1024);
    expect(PM.Memory.setBudget('preview', 3 * 1024 * 1024)).toBe(true);
    expect(PM.Memory.stats().preview.bytes).toBe(3 * 1024 * 1024);
    expect(PM.Memory.setBudget('preview', 0)).toBe(true);
    expect(PM.Memory.stats().preview.bytes).toBe(0);
    expect(PM.Memory.setBudget('raster', 0)).toBe(false);
  });
  it('counts and cancels invocation-owned video input previews when the shared cap is lowered or pressured', () => {
    (globalThis as any).window = {addEventListener:vi.fn()};const PM={} as PMRegistry;install(PM);
    let bytes=8192;const cancel=vi.fn(()=>{bytes=0;});PM.GL={get inputPreviewBytes(){return bytes;},cancelInputPreview:cancel};
    expect(PM.Memory.stats().preview.bytes).toBe(8192);PM.Memory.pressure('critical');expect(cancel).toHaveBeenCalledOnce();
    bytes=8192;PM.Memory.setBudget('preview',0);expect(cancel).toHaveBeenCalledTimes(2);expect(PM.Memory.stats().preview.bytes).toBe(0);
  });

  it('measures providers and requests deterministic trimming under pressure', () => {
    (globalThis as any).window = { addEventListener: vi.fn() };
    const PM = {} as PMRegistry;
    install(PM);
    let bytes = 100;
    const trim = vi.fn((target: number) => { bytes = Math.min(bytes, target); });
    PM.Memory.register('custom', { bytes: () => bytes, entries: () => 3, trim });
    PM.Memory.setBudget('custom', 2 * 1024 * 1024);

    expect(PM.Memory.stats().custom).toEqual({ bytes: 100, budget: 2 * 1024 * 1024, entries: 3 });
    PM.Memory.pressure('critical');
    expect(trim).toHaveBeenLastCalledWith(512 * 1024);
  });

  it('defers routine eviction until idle and uses hysteresis to avoid cache thrash', () => {
    let idle: (() => void) | undefined;
    (globalThis as any).window = {
      addEventListener: vi.fn(),
      requestIdleCallback: vi.fn((action: () => void) => { idle = action; return 7; }),
      cancelIdleCallback: vi.fn(),
      setTimeout,
      clearTimeout,
    };
    const PM = {} as PMRegistry;
    install(PM);
    let bytes = 0;
    const trim = vi.fn((target: number) => { bytes = Math.min(bytes, target); });
    PM.Memory.setBudget('custom', 2 * 1024 * 1024);
    PM.Memory.register('custom', { bytes: () => bytes, entries: () => 4, trim });
    trim.mockClear();
    bytes = 2.4 * 1024 * 1024;

    expect(PM.Memory.maintain('custom')).toBe(true);
    expect(trim).not.toHaveBeenCalled();
    idle?.();
    expect(trim).toHaveBeenCalledWith(Math.floor(1.8 * 1024 * 1024));
  });
});
