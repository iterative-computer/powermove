import { afterEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './memory';

const originalWindow = (globalThis as any).window;
afterEach(() => {
  if (originalWindow === undefined) delete (globalThis as any).window;
  else (globalThis as any).window = originalWindow;
});

describe('memory budget manager', () => {
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
