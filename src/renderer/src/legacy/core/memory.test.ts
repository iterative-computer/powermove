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
});
