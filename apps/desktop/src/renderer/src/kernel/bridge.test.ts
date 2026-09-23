// @vitest-environment happy-dom
import { afterAll, describe, expect, it, vi } from 'vitest';

import type { PowermoveBridge } from '../../../shared/ipc';
import { bridge, provideBridge, resetBridgeForTests } from './bridge';
import { captureBridge } from './capture-bridge';
import { resetExtensionsStore } from './extensions.svelte';
import { bootExtensions, installKernel } from './install';
import { RUNTIME_GLOBAL } from './runtime-globals';
import { fakePM } from './__fixtures__/fake-pm';

/* One realm per file: the real window gets sealed once, in the boot test. */

afterAll(() => {
  resetBridgeForTests();
  delete (globalThis as Record<string, unknown>)[RUNTIME_GLOBAL];
});

function fakeExtensionsBridge(records: unknown[]) {
  return {
    list: vi.fn(async () => records),
    setEnabled: vi.fn(async () => []),
    remove: vi.fn(async () => []),
    reload: vi.fn(async () => []),
    create: vi.fn(async () => []),
    fork: vi.fn(async ({ id }: { id: string }) => ({ id: `${id}-fork` })),
    reveal: vi.fn(async () => undefined),
    readSource: vi.fn(async () => []),
    reportHealth: vi.fn(),
    onChanged: vi.fn(() => () => {})
  };
}

describe('captureBridge', () => {
  it('takes the bridge off its scope, pins the name to undefined, and keeps it', () => {
    resetBridgeForTests();
    const fake = { ping: async () => 'pong' } as unknown as PowermoveBridge;
    const scope: Record<string, unknown> = {};
    Object.defineProperty(scope, 'powermove', { value: fake, configurable: true, enumerable: false, writable: false });

    expect(captureBridge(scope)).toBe(fake);
    expect(scope.powermove).toBeUndefined();
    expect(bridge()).toBe(fake);
    // Nothing can put a stand-in back.
    expect(() => Object.defineProperty(scope, 'powermove', { value: {} })).toThrow(TypeError);
    expect(Reflect.set(scope, 'powermove', {})).toBe(false);
    expect(scope.powermove).toBeUndefined();
    // A second capture is a no-op.
    expect(captureBridge({ powermove: {} })).toBe(fake);
  });

  it('seals a page that never had a bridge, and takes a host-built one without touching it', () => {
    resetBridgeForTests();
    const scope: Record<string, unknown> = {};
    expect(captureBridge(scope)).toBeUndefined();
    expect(Object.getOwnPropertyDescriptor(scope, 'powermove')).toMatchObject({ value: undefined, configurable: false, writable: false });
    const served = { remote: true } as unknown as PowermoveBridge;
    provideBridge(served);
    expect(bridge()).toBe(served);
    expect(scope.powermove).toBeUndefined();
    expect(() => provideBridge(served)).toThrow(/already installed/);
  });
});

describe('renderer boot with a fake bridge', () => {
  it('boots the kernel through the capture while extensions see no window.powermove', async () => {
    resetBridgeForTests();
    resetExtensionsStore();
    const probe = encodeURIComponent([
      'export default function (api) {',
      '  globalThis.__bridgeProbe = {',
      '    windowBridge: typeof window.powermove,',
      '    globalBridge: typeof globalThis.powermove,',
      "    inKeys: Object.keys(window).includes('powermove'),",
      '    haptic: typeof api.host.haptic.alignment',
      '  };',
      '}'
    ].join('\n'));
    const extensions = fakeExtensionsBridge([{
      id: 'probe',
      scope: 'user',
      manifest: { id: 'probe', name: 'Probe', version: '1.0.0', apiVersion: 1 },
      dir: '/ext/probe',
      enabled: true,
      bundleUrl: `data:text/javascript,${probe}`,
      bundleHash: 'probe',
      health: { state: 'ok' },
      updatedAt: 0,
      trust: 'local'
    }]);
    const alignment = vi.fn();
    const fake = { extensions, haptic: { alignment } } as unknown as PowermoveBridge;
    // What the preload does: a configurable global the kernel can take away.
    const fakeWindow = Object.create(window) as Window & { powermove: PowermoveBridge };
    Object.defineProperty(fakeWindow, 'powermove', { value: fake, configurable: true });
    vi.stubGlobal('window', fakeWindow);

    captureBridge(fakeWindow);
    expect((window as unknown as { powermove?: unknown }).powermove).toBeUndefined();
    expect(bridge()).toBe(fake);

    const installed = installKernel(fakePM());
    try {
      // The kernel still reaches the bridge: forks go through it.
      await expect(installed.api('mods').extensions.fork('timeline')).resolves.toEqual({ id: 'timeline-fork' });
      expect(extensions.fork).toHaveBeenCalledExactlyOnceWith({ id: 'timeline' });

      const loader = await bootExtensions(installed, {});
      await loader.whenIdle();
      expect(extensions.list).toHaveBeenCalled();
      expect(loader.activeIds()).toContain('probe');
      expect((globalThis as { __bridgeProbe?: unknown }).__bridgeProbe).toEqual({
        windowBridge: 'undefined',
        globalBridge: 'undefined',
        inKeys: false,
        haptic: 'function'
      });

      // Built-ins get haptics through the API instead.
      installed.api('viewer').host.haptic.alignment();
      expect(alignment).toHaveBeenCalledTimes(1);
      await loader.dispose();
    } finally {
      installed.uninstall();
      vi.unstubAllGlobals();
      delete (globalThis as { __bridgeProbe?: unknown }).__bridgeProbe;
    }
  });
});
