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

function loadShim(options: { snapshotError?: Error; nativeAsyncStore?: boolean; serializedBridge?: boolean; snapshotJSON?: Record<string, string>; windows?: boolean; storeSync?: boolean } = {}) {
  const emitted: Array<[string, unknown]> = [];
  const PM: PMRegistry = {
    uid: vi.fn((prefix: string) => `${prefix}test`),
    toast: vi.fn(),
    bus: { emit: (event: string, payload: unknown) => emitted.push([event, payload]), on: vi.fn() },
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
        : vi.fn(() => ({ 'project.demo': { layers: [{ id: 'one' }] }, __powermoveAsyncStore: options.nativeAsyncStore === true })),
      ...(options.snapshotJSON !== undefined ? { snapshotSerializedSync: vi.fn(() => options.snapshotJSON) } : {}),
      ...(options.serializedBridge ? { setSerialized: vi.fn(async () => undefined) } : {}),
      set: vi.fn(),
      delete: vi.fn(),
      flush: vi.fn(),
      onError: vi.fn(),
      ...(options.storeSync
        ? {
          getSync: vi.fn((key: string) => key === 'projects' ? JSON.stringify([{ id: 'from-sibling' }]) : null),
          onChanged: vi.fn(),
        }
        : {}),
    },
    ...(options.windows
      ? {
        windows: {
          initialProject: vi.fn(() => ({ projectId: 'P1', taken: ['P2'] })),
          claimProject: vi.fn(async () => ({ claimed: false, focused: true })),
          openProject: vi.fn(async () => ({ opened: true, focused: false })),
          create: vi.fn(async () => undefined),
          close: vi.fn(),
        }
      }
      : {}),
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
  return { PM, bridge, window: shimWindow, addEventListener, emitted };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('legacy Electron shim install', () => {
  it('requires agreement from native storage and preload before migrating undo history', () => {
    expect(loadShim({ serializedBridge: true }).PM.store.separateHistory).toBe(false);
    expect(loadShim({ nativeAsyncStore: true }).PM.store.separateHistory).toBe(false);
    expect(loadShim({ nativeAsyncStore: true, serializedBridge: true }).PM.store.separateHistory).toBe(true);
  });

  it('re-reads a key a sibling window wrote, and only when it is next asked for', () => {
    const { PM, bridge, emitted } = loadShim({ storeSync: true });
    const changed = bridge.store.onChanged.mock.calls[0]![0] as (keys: string[]) => void;

    changed(['projects', 'openWindows']);
    // Invalidation is lazy: a neighbour's write costs nothing until it is read.
    expect(bridge.store.getSync).not.toHaveBeenCalled();
    expect(emitted).toContainEqual(['store:external', ['projects', 'openWindows']]);

    expect(PM.store.get('projects', null)).toEqual([{ id: 'from-sibling' }]);
    expect(bridge.store.getSync).toHaveBeenCalledWith('projects');
    // The refreshed value is cached again, so a second read is free.
    expect(PM.store.get('projects', null)).toEqual([{ id: 'from-sibling' }]);
    expect(bridge.store.getSync).toHaveBeenCalledTimes(1);

    // A key the store no longer holds falls back rather than serving a stale copy.
    expect(PM.store.get('openWindows', 'fallback')).toBe('fallback');

    // This window's own write settles the key; nothing is re-read behind it.
    bridge.store.getSync.mockClear();
    changed(['projects']);
    PM.store.set('projects', [{ id: 'mine' }]);
    expect(PM.store.get('projects', null)).toEqual([{ id: 'mine' }]);
    expect(bridge.store.getSync).not.toHaveBeenCalled();
  });

  it('routes window work through the native bridge and degrades without one', async () => {
    const { PM, bridge } = loadShim({ windows: true });
    expect(PM.windows.supported).toBe(true);
    expect(PM.windows.initialProject()).toEqual({ projectId: 'P1', taken: ['P2'] });
    // A refused claim is reported as-is: the caller keeps the document it had.
    await expect(PM.windows.claimProject('P2')).resolves.toEqual({ claimed: false, focused: true });
    await expect(PM.windows.openProject('P2')).resolves.toEqual({ opened: true, focused: false });
    await expect(PM.windows.create()).resolves.toBe(true);
    PM.windows.close();
    expect(bridge.windows.close).toHaveBeenCalledOnce();

    // Without the bridge a claim still succeeds, so a lone window keeps working.
    const plain = loadShim();
    expect(plain.PM.windows.supported).toBe(false);
    expect(plain.PM.windows.initialProject()).toEqual({ projectId: null, taken: [] });
    await expect(plain.PM.windows.claimProject('P1')).resolves.toEqual({ claimed: true, focused: false });
    await expect(plain.PM.windows.create()).resolves.toBe(false);
  });

  it('flushes the newest asynchronous history snapshot before native storage', async () => {
    const { PM, bridge } = loadShim({ nativeAsyncStore: true, serializedBridge: true });
    void PM.store.setAsync('projectHistory.demo', { entries: ['older'] });
    void PM.store.setAsync('projectHistory.demo', { entries: ['newest'] });
    await PM.store.flush();
    expect(bridge.store.setSerialized).toHaveBeenCalledTimes(1);
    expect(bridge.store.setSerialized).toHaveBeenCalledWith('projectHistory.demo', '{"entries":["newest"]}');
    expect(bridge.store.flush).toHaveBeenCalledOnce();
  });

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
      return {
        ok: true,
        text: 'Finished ✓',
        access: 'editor',
        extensionChangeSetId: 'run-restore-1',
        liveEditsApplied: true,
        liveEditHistoryId: 'native-history-1'
      };
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
      reasoningEffort: 'max',
    });
    const progress = (PM.CodexBridge.progress as ReturnType<typeof vi.fn>).mock.calls[0]?.[1] as {
      dataBase64: string;
    };
    const result = (PM.CodexBridge.resolve as ReturnType<typeof vi.fn>).mock.calls[0]?.[1] as {
      ok: boolean;
      dataBase64: string;
      extensionChangeSetId?: string;
      liveEditsApplied?: boolean;
      liveEditHistoryId?: string;
    };
    expect(decodeBase64(progress.dataBase64)).toBe('Researching on the web…');
    expect(PM.CodexBridge.trace).toHaveBeenCalledExactlyOnceWith('request-1234', {
      kind: 'tool-start', itemId: 'search-1', toolName: 'search', label: 'search · motion'
    });
    expect(result.ok).toBe(true);
    expect(decodeBase64(result.dataBase64)).toBe('Finished ✓');
    expect(result.extensionChangeSetId).toBe('run-restore-1');
    expect(result.liveEditsApplied).toBe(true);
    expect(result.liveEditHistoryId).toBe('native-history-1');

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
      reasoningEffort: 'xhigh',
    });
  });

  it('uses the connected ChatGPT subscription and Luna for title-only turns', async () => {
    const { PM, bridge } = loadShim();
    bridge.codex.run.mockResolvedValue({
      ok: true, text: '{"title":"Premiere-style timeline"}', access: 'editor'
    });

    await expect(PM.AgentThreadTitles.generate('Arrange this timeline like Premiere Pro')).resolves.toBe(
      '{"title":"Premiere-style timeline"}'
    );
    expect(bridge.codex.run).toHaveBeenCalledWith(expect.objectContaining({
      provider: 'chatgpt', model: 'gpt-5.6-luna', reasoningEffort: 'low',
      mode: 'editor', access: 'editor', projectJSON: null, images: [], attachments: [],
    }));
    expect(bridge.codex.run.mock.calls[0]?.[0].prompt).toContain('Arrange this timeline like Premiere Pro');
  });

  it('uses the connected Claude subscription and Haiku for Claude title-only turns', async () => {
    const { PM, bridge } = loadShim();
    bridge.codex.run.mockResolvedValue({
      ok: true, text: '{"title":"Premiere-style timeline"}', access: 'editor'
    });

    await PM.AgentThreadTitles.generate('Arrange this timeline like Premiere Pro', 'claude');
    expect(bridge.codex.run).toHaveBeenCalledWith(expect.objectContaining({
      provider: 'claude', model: 'haiku', reasoningEffort: 'low',
      mode: 'editor', access: 'editor', projectJSON: null,
    }));
  });

  it('passes artifact bytes directly and preserves window-capture base64', async () => {
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
      data: new Uint8Array([104, 105]),
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

  it('restores serialized recovery without requesting the bridged object tree', () => {
    const history = { undo: [{ layers: [{ id: 'one', name: 'Restored' }] }], redo: [] };
    const { PM, bridge } = loadShim({ snapshotJSON: {
      'projectHistory.demo': JSON.stringify(history), takes: JSON.stringify([{ name: 'Original', json: '{"layers":[]}' }]), __powermoveAsyncStore: 'true',
    }, serializedBridge: true });
    expect(bridge.store.snapshotSync).not.toHaveBeenCalled();
    expect(PM.store.get('projectHistory.demo')).toEqual(history);
    expect(PM.store.get('takes')).toEqual([{ name: 'Original', json: '{"layers":[]}' }]);
    expect(PM.store.separateHistory).toBe(true);
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
