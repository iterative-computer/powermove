import { afterEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './electron-shim';

function loadShim() {
  const PM: PMRegistry = {
    toast: vi.fn(),
    CodexBridge: { progress: vi.fn(), resolve: vi.fn() },
    AgentArtifacts: { resolve: vi.fn() },
    WindowCapture: { resolve: vi.fn() },
    store: {},
  };
  const bridge: any = {
    saveFile: vi.fn(),
    codex: {
      run: vi.fn(),
      cancel: vi.fn(),
      requestComputerConsent: vi.fn(),
    },
    artifacts: { read: vi.fn(), reveal: vi.fn() },
    captureWindow: vi.fn(),
    store: {
      snapshotSync: vi.fn(() => ({ 'project.demo': { layers: [{ id: 'one' }] } })),
      set: vi.fn(),
      delete: vi.fn(),
      onError: vi.fn(),
    },
    setTheme: vi.fn(),
    log: vi.fn(),
    onMenuCommand: vi.fn(),
  };
  const addEventListener = vi.fn();
  const shimWindow: any = {
    powermove: bridge,
    TextEncoder,
    atob: (value: string) => Buffer.from(value, 'base64').toString('binary'),
    btoa: (value: string) => Buffer.from(value, 'binary').toString('base64'),
    structuredClone,
    console,
    document: {
      readyState: 'loading',
      addEventListener,
      documentElement: { classList: { add: vi.fn() } },
    },
    addEventListener: vi.fn(),
    setTimeout,
  };
  vi.stubGlobal('window', shimWindow);

  install(PM);
  return { PM, bridge, window: shimWindow, addEventListener };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('legacy Electron shim install', () => {
  it('maps legacy store keys, detaches values, and retains the boot hook', () => {
    const { PM, bridge, window, addEventListener } = loadShim();
    const store = PM.store as any;

    const first = store.get('project.demo');
    first.layers.push({ id: 'mutated' });
    expect(store.get('pm.project.demo')).toEqual({ layers: [{ id: 'one' }] });

    const value = { layers: [{ id: 'saved' }] };
    store.set('pm.project.demo', value);
    value.layers.push({ id: 'later' });
    expect(bridge.store.set).toHaveBeenCalledWith('project.demo', { layers: [{ id: 'saved' }] });
    expect(store.get('project.demo')).toEqual({ layers: [{ id: 'saved' }] });

    const onError = bridge.store.onError.mock.calls[0][0];
    store.set('not-registered', { phantom: true });
    onError({ key: 'not-registered', error: 'unknown store key: not-registered' });
    expect(store.get('not-registered', null)).toBeNull();
    expect(window.webkit.messageHandlers.pmLog.postMessage).toBeTypeOf('function');
    expect(addEventListener).toHaveBeenCalledWith('DOMContentLoaded', expect.any(Function), { once: true });
  });
});
