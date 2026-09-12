import { describe, expect, it, vi } from 'vitest';

import { idlePreload } from './idle-preload';

describe('idle optional-feature preload', () => {
  it('starts during idle and reuses the loaded module on first interaction', async () => {
    let idle: (() => void) | undefined;
    const host = {
      requestIdleCallback: vi.fn((action: () => void) => { idle = action; return 1; }),
      cancelIdleCallback: vi.fn(),
    };
    const module = { default: 'component' };
    const loader = vi.fn(async () => module);
    const preload = idlePreload(host, loader);

    expect(loader).not.toHaveBeenCalled();
    idle?.();
    expect(await preload.start()).toBe(module);
    expect(loader).toHaveBeenCalledOnce();
  });

  it('starts immediately on interaction when the idle callback has not run', async () => {
    const host = {
      requestIdleCallback: vi.fn(() => 9),
      cancelIdleCallback: vi.fn(),
    };
    const loader = vi.fn(async () => ({ default: 'component' }));
    const preload = idlePreload(host, loader);

    await preload.start();
    expect(host.cancelIdleCallback).toHaveBeenCalledWith(9);
    expect(loader).toHaveBeenCalledOnce();
  });
});
