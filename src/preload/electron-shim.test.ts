import { readFileSync } from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';

import { describe, expect, it, vi } from 'vitest';

const source = readFileSync(path.join(process.cwd(), 'host/electron-shim.js'), 'utf8');

function flush(): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, 0));
}

function decodeBase64(value: string): string {
  return Buffer.from(value, 'base64').toString('utf8');
}

function loadShim(options: { snapshotError?: Error } = {}) {
  const PM = {
    toast: vi.fn(),
    CodexBridge: { progress: vi.fn(), resolve: vi.fn() },
    AgentArtifacts: { resolve: vi.fn() },
    WindowCapture: { resolve: vi.fn() },
    store: {}
  };
  const bridge = {
    saveFile: vi.fn(),
    codex: {
      run: vi.fn(),
      cancel: vi.fn(),
      requestComputerConsent: vi.fn()
    },
    artifacts: { read: vi.fn(), reveal: vi.fn() },
    captureWindow: vi.fn(),
    store: {
      snapshotSync: options.snapshotError
        ? vi.fn(() => { throw options.snapshotError; })
        : vi.fn(() => ({ 'project.demo': { layers: [{ id: 'one' }] } })),
      snapshot: vi.fn(),
      set: vi.fn(),
      delete: vi.fn(),
      flush: vi.fn(),
      onError: vi.fn()
    },
    setTheme: vi.fn(),
    log: vi.fn(),
    onMenuCommand: vi.fn()
  };
  const document = {
    readyState: 'loading',
    addEventListener: vi.fn(),
    documentElement: { classList: { add: vi.fn() } }
  };
  const window = { PM, powermove: bridge } as Record<string, unknown>;
  const context = vm.createContext({
    window,
    document,
    console,
    TextEncoder,
    TextDecoder,
    Uint8Array,
    structuredClone,
    atob,
    btoa,
    addEventListener: vi.fn(),
    setTimeout,
    clearTimeout
  });

  vm.runInContext(source, context, { filename: 'host/electron-shim.js' });
  return { PM, bridge, window };
}

describe('Electron legacy renderer shim', () => {
  it('maps Codex authority and effort and emits plain UTF-8 progress/result payloads', async () => {
    const { PM, bridge, window } = loadShim();
    bridge.codex.run.mockImplementation(async (_request, onProgress) => {
      onProgress?.('Researching on the web…');
      return { ok: true, text: 'Finished ✓', access: 'editor' };
    });

    const handlers = (window.webkit as {
      messageHandlers: Record<string, { postMessage(body: unknown): void }>;
    }).messageHandlers;
    handlers.pmCodex?.postMessage({
      id: 'request-1234',
      mode: 'editor',
      access: 'computer',
      reasoningEffort: 'max',
      prompt: 'Animate it'
    });
    await flush();

    expect(bridge.codex.run).toHaveBeenCalledOnce();
    expect(bridge.codex.run.mock.calls[0]?.[0]).toMatchObject({
      mode: 'editor',
      access: 'editor',
      reasoningEffort: 'high'
    });
    const progress = PM.CodexBridge.progress.mock.calls[0]?.[1] as { dataBase64: string };
    const result = PM.CodexBridge.resolve.mock.calls[0]?.[1] as {
      ok: boolean;
      dataBase64: string;
    };
    expect(decodeBase64(progress.dataBase64)).toBe('Researching on the web…');
    expect(result.ok).toBe(true);
    expect(decodeBase64(result.dataBase64)).toBe('Finished ✓');

    handlers.pmCodex?.postMessage({
      id: 'request-5678',
      mode: 'autonomous',
      access: 'editor',
      reasoningEffort: 'xhigh',
      prompt: 'Inspect it'
    });
    await flush();
    expect(bridge.codex.run.mock.calls[1]?.[0]).toMatchObject({
      mode: 'autonomous',
      access: 'project',
      reasoningEffort: 'high'
    });
  });

  it('preserves exact artifact and window-capture base64 result shapes', async () => {
    const { PM, bridge, window } = loadShim();
    bridge.artifacts.read.mockResolvedValue({
      name: 'report.txt',
      mime: 'text/plain',
      data: new Uint8Array([104, 105])
    });
    bridge.captureWindow.mockResolvedValue(new Uint8Array([137, 80, 78, 71]));
    const handlers = (window.webkit as {
      messageHandlers: Record<string, { postMessage(body: unknown): void }>;
    }).messageHandlers;

    handlers.pmAgentArtifact?.postMessage({ id: 'artifact-1', projectId: 'demo', path: 'run/a' });
    handlers.pmCaptureWindow?.postMessage({ id: 'capture-1' });
    await flush();

    expect(PM.AgentArtifacts.resolve).toHaveBeenCalledWith('artifact-1', {
      ok: true,
      name: 'report.txt',
      mime: 'text/plain',
      dataBase64: 'aGk='
    });
    expect(PM.WindowCapture.resolve).toHaveBeenCalledWith('capture-1', {
      ok: true,
      dataBase64: 'iVBORw=='
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
    const handlers = (window.webkit as {
      messageHandlers: Record<string, { postMessage(body: unknown): void }>;
    }).messageHandlers;

    expect(bridge.log).toHaveBeenCalledWith('error', 'store snapshot failed: channel unavailable');
    handlers.pmLog?.postMessage(`[warn] ${'x'.repeat(9000)}`);
    expect(bridge.log).toHaveBeenLastCalledWith('warn', expect.any(String));
    expect((bridge.log.mock.calls.at(-1)?.[1] as string)).toHaveLength(8000);
  });
});
