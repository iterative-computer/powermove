import type { IpcMain, IpcMainInvokeEvent } from 'electron';
import { describe, expect, it, vi } from 'vitest';

import { ENGINE_IPC } from '../../shared/engine-ipc';
import type { EngineProxyService } from './proxies';
import { installProxyIpc, proxyDescriptors } from './proxy-ipc';

function install(trusted = true) {
  let handler: ((event: IpcMainInvokeEvent, request: unknown) => Promise<unknown>) | null = null;
  const ipc = { handle: vi.fn((_c: string, h: typeof handler) => { handler = h; }), removeHandler: vi.fn() } as unknown as IpcMain;
  const service = {
    snapshot: vi.fn(async () => 'snapshot'), setIntent: vi.fn(async () => undefined), create: vi.fn(async () => undefined),
    rebuild: vi.fn(async () => undefined), remove: vi.fn(async () => undefined), slow: vi.fn(async () => undefined),
    location: vi.fn(async () => ({ kind: 'cache' })), setLocation: vi.fn(async (l: unknown) => l)
  };
  const chooseFolder = vi.fn(async () => '/Volumes/Media');
  const remove = installProxyIpc({ ipc, trusted: () => trusted, service: service as unknown as EngineProxyService, chooseFolder });
  const call = (request: unknown, sender = 9) => handler!({ sender: { id: sender } } as IpcMainInvokeEvent, request);
  return { ipc, service, chooseFolder, call, remove };
}
const intent = { create: 'allVideo', size: 'half', format: 'proResLt', alwaysPlayOriginal: ['a2'] };

describe('installProxyIpc', () => {
  it('rejects untrusted senders before reading the request', async () => {
    const { call, service } = install(false);
    await expect(call({ action: 'location' })).rejects.toThrow('Unauthorized IPC sender');
    expect(service.location).not.toHaveBeenCalled();
  });

  it('routes actions with the sender as owner and removes its handler', async () => {
    const { call, service, chooseFolder, ipc, remove } = install();
    expect(await call({ action: 'snapshot', project: 'p1' }, 4)).toBe('snapshot');
    expect(service.snapshot).toHaveBeenCalledWith(4, 'p1');
    await call({ action: 'create', project: 'p1', assets: ['a1'] });
    await call({ action: 'rebuild', project: 'p1', assets: ['a1'] });
    await call({ action: 'remove', project: 'p1', assets: ['a1'] });
    await call({ action: 'slow', project: 'p1', assets: ['a1'] });
    expect(service.create).toHaveBeenCalledWith(9, 'p1', ['a1']);
    expect(service.rebuild).toHaveBeenCalledWith(9, 'p1', ['a1']);
    expect(service.remove).toHaveBeenCalledWith(9, 'p1', ['a1']);
    expect(service.slow).toHaveBeenCalledWith(9, 'p1', ['a1']);
    expect(await call({ action: 'chooseFolder' })).toBe('/Volumes/Media');
    expect(chooseFolder).toHaveBeenCalledOnce();
    await expect(call({ action: 'explode', project: 'p1', assets: [] })).rejects.toThrow('Unknown proxy action.');
    remove();
    expect(ipc.removeHandler).toHaveBeenCalledWith(ENGINE_IPC.proxies);
  });

  it('validates the intent, drops renderer paths and bad hashes, and only passes absolute project paths', async () => {
    const { call, service } = install();
    const assets = {
      a1: { contentHash: 'a'.repeat(64), kind: 'video', name: 'Clip', sourcePath: '/etc/passwd', imageSequence: true, alwaysPlayOriginal: true, extra: 1 },
      a2: { contentHash: 'A'.repeat(64), kind: 'video' }
    };
    await call({ action: 'setIntent', project: 'p1', intent, assets, projectPath: '/Users/x/Film.pmv' });
    expect(service.setIntent).toHaveBeenLastCalledWith(9, 'p1', intent, {
      a1: { contentHash: 'a'.repeat(64), kind: 'video', name: 'Clip', imageSequence: true, alwaysPlayOriginal: true },
      a2: { kind: 'video' }
    }, '/Users/x/Film.pmv');
    await call({ action: 'setIntent', project: 'p1', intent, assets: {}, projectPath: 'relative.pmv' });
    expect(service.setIntent).toHaveBeenLastCalledWith(9, 'p1', intent, {}, undefined);
    for (const bad of [{ ...intent, create: 'always' }, { ...intent, size: 'p4k' }, { ...intent, format: 'h264' }, { ...intent, alwaysPlayOriginal: [''] }, null]) {
      await expect(call({ action: 'setIntent', project: 'p1', intent: bad, assets: {} })).rejects.toThrow('Invalid project proxy settings.');
    }
    await expect(call({ action: 'setIntent', project: 'p1', intent, assets: { a: 'video' } })).rejects.toThrow('Invalid proxy asset.');
    await expect(call({ action: 'create', project: '', assets: [] })).rejects.toThrow('names an open project');
    await expect(call({ action: 'create', project: 'p1', assets: [7] })).rejects.toThrow('names assets');
    expect(service.create).not.toHaveBeenCalled();
  });

  it('accepts only valid generated-media locations', async () => {
    const { call, service } = install();
    await call({ action: 'setLocation', location: { kind: 'folder', path: '/Volumes/Media' } });
    expect(service.setLocation).toHaveBeenCalledWith({ kind: 'folder', path: '/Volumes/Media' });
    for (const bad of [{ kind: 'folder', path: 'relative' }, { kind: 'folder', path: '/a\0b' }, { kind: 'cache', path: '/x' }, { kind: 'cloud' }, 'cache']) {
      await expect(call({ action: 'setLocation', location: bad })).rejects.toThrow('Choose a generated media location.');
    }
    expect(service.setLocation).toHaveBeenCalledOnce();
  });

  it('caps descriptor counts', () => {
    const many = Object.fromEntries(Array.from({ length: 100_001 }, (_, i) => [`a${i}`, {}]));
    expect(() => proxyDescriptors(many)).toThrow('Invalid project proxy settings.');
  });
});
