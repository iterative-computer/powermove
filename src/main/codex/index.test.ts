import { EventEmitter } from 'node:events';

import { beforeEach, describe, expect, it, vi } from 'vitest';

import { IPC, type ChatGPTAccountStatus, type CodexRunRequest } from '../../shared/ipc';

const mocks = vi.hoisted(() => {
  const cancel = vi.fn(async () => true);
  const cancelAll = vi.fn(async () => undefined);
  const accountStatus = vi.fn(async () => ({
    state: 'connected' as const,
    email: 'editor@example.com',
    planType: 'plus',
    detail: null
  }));
  const accountConnect = vi.fn(async () => ({
    state: 'connecting' as const,
    email: null,
    planType: null,
    detail: 'Finish signing in in your browser.'
  }));
  const accountDisconnect = vi.fn(async () => ({
    state: 'disconnected' as const,
    email: null,
    planType: null,
    detail: null
  }));
  const accountShutdown = vi.fn(async () => undefined);
  let accountChanged: ((status: ChatGPTAccountStatus) => void) | null = null;
  let resolveRun: ((value: unknown) => void) | null = null;
  const run = vi.fn(async (_req: unknown, options: {
    onProgress?: (text: string) => void;
    onTrace?: (step: unknown) => void;
  }) => {
    options.onProgress?.('Working…');
    options.onTrace?.({ kind: 'thought', text: 'Inspecting source' });
    return await new Promise((resolve) => { resolveRun = resolve; });
  });
  return {
    appOnce: vi.fn(),
    cancel,
    cancelAll,
    accountStatus,
    accountConnect,
    accountDisconnect,
    accountShutdown,
    account: {
      status: accountStatus,
      connect: accountConnect,
      disconnect: accountDisconnect,
      shutdown: accountShutdown,
      onChanged(listener: (status: ChatGPTAccountStatus) => void) {
        accountChanged = listener;
        return () => { accountChanged = null; };
      }
    },
    emitAccount(status: ChatGPTAccountStatus) { accountChanged?.(status); },
    run,
    resolve(value: unknown) {
      resolveRun?.(value);
      resolveRun = null;
    }
  };
});

const apiPackFiles = async () => [{ name: 'EXTENSIONS.md', text: '# Extensions' }];

vi.mock('electron', () => ({ app: { once: mocks.appOnce } }));
vi.mock('./runner', () => ({
  CodexRunner: class {
    run = mocks.run;
    cancel = mocks.cancel;
    cancelAll = mocks.cancelAll;
  },
  isCodexRunRequest: () => true
}));

import { registerCodexIpc } from './index';

class Sender extends EventEmitter {
  readonly send = vi.fn();
  private destroyed = false;

  isDestroyed(): boolean { return this.destroyed; }
  destroy(): void {
    this.destroyed = true;
    this.emit('destroyed');
  }
}

function runRequest(): CodexRunRequest {
  return {
    id: 'ipc-run-1234',
    mode: 'editor',
    prompt: 'Polish it',
    schema: null,
    images: [],
    model: null,
    reasoningEffort: null,
    access: 'editor',
    projectId: 'ipc-project',
    projectName: 'IPC Project',
    projectJSON: null,
    attachments: [],
    consentToken: null
  };
}

describe('registerCodexIpc', () => {
  const handlers = new Map<string, (event: { sender: Sender }, request: unknown) => Promise<unknown>>();
  const ipcMain = {
    handle: vi.fn((channel: string, handler: (event: { sender: Sender }, request: unknown) => Promise<unknown>) => {
      handlers.set(channel, handler);
    })
  };

  beforeEach(() => {
    handlers.clear();
    vi.clearAllMocks();
    registerCodexIpc(ipcMain as never, {
      getWindow: () => null,
      userData: '/tmp/powermove-index-test',
      extensionsDir: '/tmp/powermove-user-extensions',
      apiPackFiles,
      isTrustedSender: () => true,
      codexBinaryPref: () => null,
      openExternal: async () => undefined
    }, mocks.account);
  });

  it('registers every frozen Codex, consent, and artifact channel', () => {
    expect([...handlers.keys()]).toEqual([
      IPC.chatgptStatus,
      IPC.chatgptConnect,
      IPC.chatgptDisconnect,
      IPC.codexRun,
      IPC.codexCancel,
      IPC.codexFixPrompt,
      IPC.consentComputer,
      IPC.artifactRead,
      IPC.artifactReveal
    ]);
    expect(mocks.appOnce).toHaveBeenCalledWith('before-quit', expect.any(Function));
    const beforeQuit = mocks.appOnce.mock.calls.at(-1)?.[1] as (() => void) | undefined;
    beforeQuit?.();
    expect(mocks.cancelAll).toHaveBeenCalledOnce();
    expect(mocks.accountShutdown).toHaveBeenCalledOnce();
  });

  it('reports, connects, and disconnects ChatGPT through trusted IPC', async () => {
    const sender = new Sender();
    await expect(handlers.get(IPC.chatgptStatus)!({ sender }, undefined)).resolves.toMatchObject({
      state: 'connected',
      planType: 'plus'
    });
    await expect(handlers.get(IPC.chatgptConnect)!({ sender }, undefined)).resolves.toMatchObject({
      state: 'connecting'
    });
    await expect(handlers.get(IPC.chatgptDisconnect)!({ sender }, undefined)).resolves.toMatchObject({
      state: 'disconnected'
    });
    expect(mocks.accountStatus).toHaveBeenCalledOnce();
    expect(mocks.accountConnect).toHaveBeenCalledOnce();
    expect(mocks.accountDisconnect).toHaveBeenCalledOnce();
  });

  it('streams progress and only allows the owning WebContents to cancel', async () => {
    const owner = new Sender();
    const stranger = new Sender();
    const pending = handlers.get(IPC.codexRun)!({ sender: owner }, runRequest());

    expect(mocks.run).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'ipc-run-1234' }),
      expect.objectContaining({
        extensionsDir: '/tmp/powermove-user-extensions',
        apiPackFiles
      })
    );

    await handlers.get(IPC.codexCancel)!({ sender: stranger }, { id: 'ipc-run-1234' });
    expect(mocks.cancel).not.toHaveBeenCalled();
    await handlers.get(IPC.codexCancel)!({ sender: owner }, { id: 'ipc-run-1234' });
    expect(mocks.cancel).toHaveBeenCalledWith('ipc-run-1234');
    expect(owner.send).toHaveBeenCalledWith(IPC.codexEvent, {
      id: 'ipc-run-1234',
      kind: 'progress',
      text: 'Working…'
    });
    expect(owner.send).toHaveBeenCalledWith(IPC.codexEvent, {
      id: 'ipc-run-1234',
      kind: 'trace',
      step: { kind: 'thought', text: 'Inspecting source' }
    });

    mocks.resolve({ ok: true, text: '{}', access: 'editor' });
    await expect(pending).resolves.toEqual({ ok: true, text: '{}', access: 'editor' });
  });

  it('cancels a run when its renderer is destroyed', async () => {
    const owner = new Sender();
    const pending = handlers.get(IPC.codexRun)!({ sender: owner }, runRequest());
    owner.destroy();
    expect(mocks.cancel).toHaveBeenCalledWith('ipc-run-1234');
    mocks.resolve({ ok: false, error: 'cancelled', cancelled: true });
    await pending;
  });

  it('builds a fix prompt from a validated extension failure', async () => {
    await expect(handlers.get(IPC.codexFixPrompt)!({ sender: new Sender() }, {
      id: 'broken-extension',
      error: 'index.ts:1: failed',
      files: [{ path: 'index.ts', text: 'throw new Error();' }]
    })).resolves.toContain('The extension `broken-extension` fails: index.ts:1: failed. Files:');
  });

  it.each([
    ['invalid id', { id: '../escape', error: 'failed', files: [] }],
    ['oversized error', { id: 'broken-extension', error: 'x'.repeat(4_001), files: [] }],
  ])('rejects fix-prompt requests with %s', async (_label, request) => {
    await expect(
      handlers.get(IPC.codexFixPrompt)!({ sender: new Sender() }, request)
    ).rejects.toThrow(IPC.codexFixPrompt);
  });

  it('degrades oversized fix-prompt requests instead of rejecting them', async () => {
    const many = await handlers.get(IPC.codexFixPrompt)!({ sender: new Sender() }, {
      id: 'broken-extension',
      error: 'failed',
      files: Array.from({ length: 41 }, (_, index) => ({ path: `${index}.ts`, text: '' }))
    });
    expect(many).toContain('path="39.ts"');
    expect(many).not.toContain('path="40.ts"');

    const big = await handlers.get(IPC.codexFixPrompt)!({ sender: new Sender() }, {
      id: 'broken-extension',
      error: 'failed',
      files: [{ path: 'index.ts', text: 'é'.repeat(32_769) }]
    });
    expect(big).toContain('truncated');
    expect(String(big).length).toBeLessThan(200_000);
  });

  it('rejects an untrusted sender before handling payloads', async () => {
    handlers.clear();
    registerCodexIpc(ipcMain as never, {
      getWindow: () => null,
      userData: '/tmp/powermove-index-test',
      extensionsDir: '/tmp/powermove-user-extensions',
      apiPackFiles,
      isTrustedSender: () => false,
      codexBinaryPref: () => null,
      openExternal: async () => undefined
    }, mocks.account);
    await expect(
      handlers.get(IPC.codexCancel)!({ sender: new Sender() }, { id: 'ipc-run-1234' })
    ).rejects.toThrow('Unauthorized IPC sender');
  });
});
