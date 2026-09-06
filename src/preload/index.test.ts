import { beforeEach, describe, expect, it, vi } from 'vitest';

import { IPC, type CodexRunRequest, type PowermoveBridge } from '../shared/ipc';

const electronMocks = vi.hoisted(() => ({
  bridge: undefined as PowermoveBridge | undefined,
  invoke: vi.fn(),
  on: vi.fn(),
  removeListener: vi.fn(),
  send: vi.fn(),
  sendSync: vi.fn(),
  getPathForFile: vi.fn()
}));

vi.mock('electron', () => ({
  contextBridge: {
    exposeInMainWorld: vi.fn((_name: string, bridge: PowermoveBridge) => {
      electronMocks.bridge = bridge;
    })
  },
  ipcRenderer: {
    invoke: electronMocks.invoke,
    on: electronMocks.on,
    removeListener: electronMocks.removeListener,
    send: electronMocks.send,
    sendSync: electronMocks.sendSync
  },
  webUtils: { getPathForFile: electronMocks.getPathForFile }
}));

await import('./index');

function bridge(): PowermoveBridge {
  if (!electronMocks.bridge) throw new Error('Preload bridge was not exposed');
  return electronMocks.bridge;
}

function request(): CodexRunRequest {
  return {
    id: 'request-1234',
    mode: 'editor',
    prompt: 'Make it move',
    schema: null,
    images: [],
    model: null,
    reasoningEffort: null,
    access: 'editor',
    projectId: 'editor',
    projectName: 'Untitled',
    projectJSON: null,
    attachments: [],
    consentToken: null
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('preload bridge', () => {
  it('isolates progress and trace by request and removes its one listener after resolve', async () => {
    const result = { ok: true as const, text: 'done', access: 'editor' as const };
    electronMocks.invoke.mockResolvedValue(result);
    const onProgress = vi.fn();
    const onTrace = vi.fn();

    const pending = bridge().codex.run(request(), onProgress, onTrace);
    expect(electronMocks.on).toHaveBeenCalledOnce();
    expect(electronMocks.on).toHaveBeenCalledWith(IPC.codexEvent, expect.any(Function));

    const listener = electronMocks.on.mock.calls[0]?.[1] as (
      event: object,
      progress: { id: string; kind: string; text?: string; step?: unknown }
    ) => void;
    listener({ sender: 'must-not-leak' }, { id: 'another-id', kind: 'progress', text: 'wrong' });
    listener({ sender: 'must-not-leak' }, { id: request().id, kind: 'progress', text: 'right' });
    listener({ sender: 'must-not-leak' }, {
      id: request().id,
      kind: 'trace',
      step: { kind: 'thought', text: 'Inspecting' }
    });

    await expect(pending).resolves.toEqual(result);
    expect(onProgress).toHaveBeenCalledExactlyOnceWith('right');
    expect(onTrace).toHaveBeenCalledExactlyOnceWith({ kind: 'thought', text: 'Inspecting' });
    expect(electronMocks.removeListener).toHaveBeenCalledExactlyOnceWith(IPC.codexEvent, listener);
  });

  it('removes the progress listener when invoke rejects', async () => {
    electronMocks.invoke.mockRejectedValue(new Error('main failed'));

    const pending = bridge().codex.run(request());
    const listener = electronMocks.on.mock.calls[0]?.[1];

    await expect(pending).rejects.toThrow('main failed');
    expect(electronMocks.removeListener).toHaveBeenCalledExactlyOnceWith(IPC.codexEvent, listener);
  });

  it('sends steering over its dedicated IPC channel', async () => {
    electronMocks.invoke.mockResolvedValue({ accepted: true });
    const steering = { id: 'request-1234', prompt: 'Continue with softer edges', images: [] };
    await expect(bridge().codex.steer(steering)).resolves.toEqual({ accepted: true });
    expect(electronMocks.invoke).toHaveBeenCalledExactlyOnceWith(IPC.codexSteer, steering);
  });

  it('brokers native agent tool requests without exposing Electron event objects', () => {
    const onRequest = vi.fn();
    const stop = bridge().agentTools.onRequest(onRequest);
    const listener = electronMocks.on.mock.calls[0]?.[1] as (...args: unknown[]) => void;
    const request = {
      runId: 'request-1234', callId: 'tool-call-1', tool: 'get_project_state',
      arguments: {}, baseRevision: 4
    };
    listener({ sender: 'must-not-leak' }, request);
    expect(onRequest).toHaveBeenCalledExactlyOnceWith(request);

    const response = {
      runId: request.runId, callId: request.callId, ok: true,
      content: [{ type: 'text' as const, text: '{"revision":4}' }]
    };
    bridge().agentTools.respond(response);
    expect(electronMocks.send).toHaveBeenCalledWith(IPC.agentToolResponse, response);
    stop();
    expect(electronMocks.removeListener).toHaveBeenCalledWith(IPC.agentToolRequest, listener);
  });

  it('strips Electron event objects and returns working unsubscribes', () => {
    const onStoreError = vi.fn();
    const stopStore = bridge().store.onError(onStoreError);
    const storeListener = electronMocks.on.mock.calls[0]?.[1] as (...args: unknown[]) => void;
    const storeError = { key: 'theme', error: 'disk full' };
    storeListener({ sender: 'must-not-leak' }, storeError);
    expect(onStoreError).toHaveBeenCalledExactlyOnceWith(storeError);
    stopStore();
    expect(electronMocks.removeListener).toHaveBeenCalledWith(IPC.storeError, storeListener);

    const onCommand = vi.fn();
    const stopMenu = bridge().onMenuCommand(onCommand);
    const menuListener = electronMocks.on.mock.calls[1]?.[1] as (...args: unknown[]) => void;
    menuListener({ sender: 'must-not-leak' }, 'save');
    expect(onCommand).toHaveBeenCalledExactlyOnceWith('save');
    stopMenu();
    expect(electronMocks.removeListener).toHaveBeenCalledWith(IPC.menuCommand, menuListener);
  });

  it('uses the synchronous store snapshot boot barrier', () => {
    electronMocks.sendSync.mockReturnValue({ theme: 'dark' });
    expect(bridge().store.snapshotSync()).toEqual({ theme: 'dark' });
    expect(electronMocks.sendSync).toHaveBeenCalledExactlyOnceWith(IPC.storeSnapshotSync);
  });

  it('resolves selected media paths only in preload and reads proxies in bounded requests', async () => {
    const file = { name: 'source.mov' } as File;
    const created = { ok: true, token: 'a'.repeat(32), type: 'video/quicktime', size: 12 } as const;
    const bytes = new Uint8Array([1, 2, 3]);
    electronMocks.getPathForFile.mockReturnValue('/Users/editor/source.mov');
    electronMocks.invoke.mockResolvedValueOnce(created).mockResolvedValueOnce(bytes).mockResolvedValueOnce(undefined);

    await expect(bridge().media.createPlaybackProxy(file)).resolves.toEqual(created);
    expect(electronMocks.invoke).toHaveBeenNthCalledWith(1, IPC.mediaProxyCreate, {
      sourcePath: '/Users/editor/source.mov',
      name: 'source.mov'
    });
    await expect(bridge().media.readPlaybackProxy('a'.repeat(32), 4, 3)).resolves.toEqual(bytes);
    expect(electronMocks.invoke).toHaveBeenNthCalledWith(2, IPC.mediaProxyRead, {
      token: 'a'.repeat(32), offset: 4, length: 3
    });
    await bridge().media.releasePlaybackProxy('a'.repeat(32));
    expect(electronMocks.invoke).toHaveBeenNthCalledWith(3, IPC.mediaProxyRelease, 'a'.repeat(32));
  });

  it('exposes ChatGPT status, connect, disconnect, and sanitized status events', async () => {
    const connected = {
      state: 'connected' as const,
      email: 'editor@example.com',
      planType: 'plus',
      detail: null
    };
    electronMocks.invoke.mockResolvedValue(connected);
    await expect(bridge().chatgpt.status()).resolves.toEqual(connected);
    expect(electronMocks.invoke).toHaveBeenCalledWith(IPC.chatgptStatus);
    await expect(bridge().chatgpt.connect()).resolves.toEqual(connected);
    expect(electronMocks.invoke).toHaveBeenCalledWith(IPC.chatgptConnect);
    await expect(bridge().chatgpt.disconnect()).resolves.toEqual(connected);
    expect(electronMocks.invoke).toHaveBeenCalledWith(IPC.chatgptDisconnect);

    const onChanged = vi.fn();
    const stop = bridge().chatgpt.onChanged(onChanged);
    const listener = electronMocks.on.mock.calls.at(-1)?.[1] as (...args: unknown[]) => void;
    listener({ sender: 'must-not-leak' }, connected);
    expect(onChanged).toHaveBeenCalledExactlyOnceWith(connected);
    stop();
    expect(electronMocks.removeListener).toHaveBeenCalledWith(IPC.chatgptChanged, listener);
  });

  it('exposes Claude status, connect, disconnect, and sanitized status events', async () => {
    const connected = {
      state: 'connected' as const,
      email: 'editor@example.com',
      planType: 'subscription',
      detail: null
    };
    electronMocks.invoke.mockResolvedValue(connected);
    await expect(bridge().claude.status()).resolves.toEqual(connected);
    expect(electronMocks.invoke).toHaveBeenCalledWith(IPC.claudeStatus);
    await expect(bridge().claude.connect()).resolves.toEqual(connected);
    expect(electronMocks.invoke).toHaveBeenCalledWith(IPC.claudeConnect);
    await expect(bridge().claude.disconnect()).resolves.toEqual(connected);
    expect(electronMocks.invoke).toHaveBeenCalledWith(IPC.claudeDisconnect);

    const onChanged = vi.fn();
    const stop = bridge().claude.onChanged(onChanged);
    const listener = electronMocks.on.mock.calls.at(-1)?.[1] as (...args: unknown[]) => void;
    listener({ sender: 'must-not-leak' }, connected);
    expect(onChanged).toHaveBeenCalledExactlyOnceWith(connected);
    stop();
    expect(electronMocks.removeListener).toHaveBeenCalledWith(IPC.claudeChanged, listener);
  });

  it('exposes alignment haptics as one-way IPC', () => {
    bridge().haptic.alignment();
    expect(electronMocks.send).toHaveBeenCalledExactlyOnceWith(IPC.hapticAlignment);
  });

  it('exposes validated native text editing as one-way IPC', () => {
    bridge().nativeEdit('paste');
    expect(electronMocks.send).toHaveBeenCalledExactlyOnceWith(IPC.nativeEdit, 'paste');
  });
});
