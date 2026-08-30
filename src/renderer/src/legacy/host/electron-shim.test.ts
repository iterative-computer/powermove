// @vitest-environment happy-dom

import { afterEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './electron-shim';

function flush(): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, 0));
}

function decodeBase64(value: string): string {
  return Buffer.from(value, 'base64').toString('utf8');
}

function loadShim(options: { snapshotError?: Error } = {}) {
  const PM: PMRegistry = {
    toast: vi.fn(),
    CodexBridge: { progress: vi.fn(), trace: vi.fn(), resolve: vi.fn() },
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
      snapshotSync: options.snapshotError
        ? vi.fn(() => { throw options.snapshotError; })
        : vi.fn(() => ({ 'project.demo': { layers: [{ id: 'one' }] } })),
      set: vi.fn(),
      delete: vi.fn(),
      flush: vi.fn(),
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
  it('connects native menu commands before DOM readiness or font discovery', () => {
    const { PM, bridge, window } = loadShim();
    expect(window.document.readyState).toBe('loading');
    expect(bridge.onMenuCommand).toHaveBeenCalledOnce();
    PM.cmd = vi.fn();
    bridge.onMenuCommand.mock.calls[0][0]('save');
    expect(PM.cmd).toHaveBeenCalledWith('save');
    bridge.onMenuCommand.mock.calls[0][0]('copy');
    bridge.onMenuCommand.mock.calls[0][0]('paste');
    expect(PM.cmd).toHaveBeenNthCalledWith(2, 'copyLayers');
    expect(PM.cmd).toHaveBeenNthCalledWith(3, 'pasteLayers');
  });
  it('maps Codex authority and effort and emits plain UTF-8 progress/result payloads', async () => {
    const { PM, bridge, window } = loadShim();
    bridge.codex.run.mockImplementation(async (
      _request: unknown,
      onProgress?: (text: string) => void,
      onTrace?: (step: unknown) => void
    ) => {
      onProgress?.('Researching on the web…');
      onTrace?.({ kind: 'tool-start', itemId: 'search-1', toolName: 'search', label: 'search · motion' });
      return { ok: true, text: 'Finished ✓', access: 'editor', extensionChangeSetId: 'run-restore-1' };
    });

    const handlers = window.webkit.messageHandlers;
    handlers.pmCodex?.postMessage({
      id: 'request-1234',
      mode: 'editor',
      access: 'computer',
      reasoningEffort: 'max',
      prompt: 'Animate it',
    });
    await flush();

    expect(bridge.codex.run).toHaveBeenCalledOnce();
    expect(bridge.codex.run.mock.calls[0]?.[0]).toMatchObject({
      mode: 'editor',
      access: 'editor',
      reasoningEffort: 'high',
    });
    const progress = (PM.CodexBridge.progress as ReturnType<typeof vi.fn>).mock.calls[0]?.[1] as {
      dataBase64: string;
    };
    const result = (PM.CodexBridge.resolve as ReturnType<typeof vi.fn>).mock.calls[0]?.[1] as {
      ok: boolean;
      dataBase64: string;
      extensionChangeSetId?: string;
    };
    expect(decodeBase64(progress.dataBase64)).toBe('Researching on the web…');
    expect(PM.CodexBridge.trace).toHaveBeenCalledExactlyOnceWith('request-1234', {
      kind: 'tool-start', itemId: 'search-1', toolName: 'search', label: 'search · motion'
    });
    expect(result.ok).toBe(true);
    expect(decodeBase64(result.dataBase64)).toBe('Finished ✓');
    expect(result.extensionChangeSetId).toBe('run-restore-1');

    handlers.pmCodex?.postMessage({
      id: 'request-5678',
      mode: 'autonomous',
      access: 'editor',
      reasoningEffort: 'xhigh',
      prompt: 'Inspect it',
    });
    await flush();
    expect(bridge.codex.run.mock.calls[1]?.[0]).toMatchObject({
      mode: 'autonomous',
      access: 'project',
      reasoningEffort: 'high',
    });
  });

  it('preserves exact artifact and window-capture base64 result shapes', async () => {
    const { PM, bridge, window } = loadShim();
    bridge.artifacts.read.mockResolvedValue({
      name: 'report.txt',
      mime: 'text/plain',
      data: new Uint8Array([104, 105]),
    });
    bridge.captureWindow.mockResolvedValue(new Uint8Array([137, 80, 78, 71]));
    const handlers = window.webkit.messageHandlers;

    handlers.pmAgentArtifact?.postMessage({ id: 'artifact-1', projectId: 'demo', path: 'run/a' });
    handlers.pmCaptureWindow?.postMessage({ id: 'capture-1' });
    await flush();

    expect(PM.AgentArtifacts.resolve).toHaveBeenCalledWith('artifact-1', {
      ok: true,
      name: 'report.txt',
      mime: 'text/plain',
      dataBase64: 'aGk=',
    });
    expect(PM.WindowCapture.resolve).toHaveBeenCalledWith('capture-1', {
      ok: true,
      dataBase64: 'iVBORw==',
    });
  });

  it('maps legacy keys and returns detached store values', () => {
    const { PM, bridge } = loadShim();
    const store = PM.store as {
      get(key: string, fallback?: unknown): { layers: Array<{ id: string }> };
      set(key: string, value: unknown): void;
    };

    const first = store.get('project.demo');
    first.layers.push({ id: 'mutated' });
    expect(store.get('pm.project.demo')).toEqual({ layers: [{ id: 'one' }] });

    const value = { layers: [{ id: 'saved' }] };
    store.set('pm.project.demo', value);
    value.layers.push({ id: 'later' });
    expect(bridge.store.set).toHaveBeenCalledWith('project.demo', { layers: [{ id: 'saved' }] });
    expect(store.get('project.demo')).toEqual({ layers: [{ id: 'saved' }] });

    store.set('not-registered', { phantom: true });
    const onError = bridge.store.onError.mock.calls[0]?.[0] as (event: {
      key: string;
      error: string;
    }) => void;
    onError({ key: 'not-registered', error: 'unknown store key: not-registered' });
    expect(store.get('not-registered', null)).toBeNull();
  });

  it('survives a failed sync snapshot and preserves bounded log severity', () => {
    const { bridge, window } = loadShim({ snapshotError: new Error('channel unavailable') });
    const handlers = window.webkit.messageHandlers;

    expect(bridge.log).toHaveBeenCalledWith('error', 'store snapshot failed: channel unavailable');
    handlers.pmLog?.postMessage(`[warn] ${'x'.repeat(9000)}`);
    expect(bridge.log).toHaveBeenLastCalledWith('warn', expect.any(String));
    expect(bridge.log.mock.calls.at(-1)?.[1] as string).toHaveLength(8000);
  });

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
