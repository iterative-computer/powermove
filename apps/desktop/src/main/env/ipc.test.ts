import * as fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('electron', () => ({
  BrowserWindow: { fromWebContents: vi.fn(() => null) },
  clipboard: { writeText: vi.fn() },
  dialog: { showMessageBox: vi.fn(async () => ({ response: 1 })) }
}));

import type { ExtensionRecord } from '../../shared/extensions';
import { VARS_IPC, type VarsStatus } from '../../shared/vars-ipc';
import { createProvenanceStore } from '../cloud/provenance';
import { createRemoveValuesPrompt, maskValue, registerVarsIpc } from './ipc';
import { createVarsService } from './service';
import { createEnvStore, type SafeStorageLike } from './store';

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map((dir) => fs.rm(dir, { recursive: true, force: true })));
});

const safeStorage: SafeStorageLike = {
  isEncryptionAvailable: () => true,
  encryptString: (text) => Buffer.from(`sealed:${text}`),
  decryptString: (buffer) => buffer.toString().slice('sealed:'.length)
};

function record(id: string, over: Partial<ExtensionRecord> = {}): ExtensionRecord {
  return {
    id,
    scope: 'user',
    manifest: {
      id,
      name: 'Colour match',
      version: '1.0.0',
      apiVersion: 2,
      vars: [
        { key: 'API_KEY', label: 'OpenAI API key', secret: true, required: true },
        { key: 'REGION', label: 'Region' }
      ]
    },
    dir: `/ext/${id}`,
    enabled: true,
    bundleUrl: null,
    bundleHash: null,
    health: { state: 'needs-setup', missing: ['API_KEY'] },
    updatedAt: 0,
    ...over
  };
}

async function harness(options: { trusted?: boolean; response?: number } = {}) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'powermove-vars-ipc-'));
  roots.push(root);
  const env = createEnvStore({ dir: path.join(root, 'env'), safeStorage });
  const provenance = createProvenanceStore(root);
  const vars = createVarsService({ env, provenance });
  const records = [record('colour-match'), record('builtin-one', { scope: 'builtin' })];
  const registry = {
    list: vi.fn(() => records.map((item) => structuredClone(item))),
    refresh: vi.fn(async () => undefined),
    emitChanged: vi.fn()
  };
  const handlers = new Map<string, (event: unknown, payload: unknown) => Promise<unknown>>();
  const ipcMain = { handle: (channel: string, handler: (event: unknown, payload: unknown) => Promise<unknown>) => void handlers.set(channel, handler) };
  const showMessageBox = vi.fn(async () => ({ response: options.response ?? 1 }));
  const writeClipboard = vi.fn();
  const event = { sender: {}, senderFrame: null };
  registerVarsIpc(ipcMain as never, {
    registry,
    vars,
    isTrusted: () => options.trusted ?? true,
    showMessageBox,
    writeClipboard,
    windowFor: () => null
  });
  const call = <T>(channel: string, payload: unknown): Promise<T> => handlers.get(channel)!(event, payload) as Promise<T>;
  return { root, env, provenance, vars, registry, handlers, call, showMessageBox, writeClipboard, event };
}

describe('vars IPC', () => {
  it('rejects malformed payloads before touching anything', async () => {
    const h = await harness();
    const bad: Array<[string, unknown]> = [
      [VARS_IPC.status, null],
      [VARS_IPC.status, { id: '../escape' }],
      [VARS_IPC.status, { id: 'colour-match', extra: true }],
      [VARS_IPC.set, { id: 'colour-match', key: 'lowercase', value: 'x' }],
      [VARS_IPC.set, { id: 'colour-match', key: 'API_KEY', value: 42 }],
      [VARS_IPC.set, { id: 'colour-match', key: 'API_KEY', value: 'x'.repeat(8193) }],
      [VARS_IPC.set, { id: 'colour-match', key: 'API_KEY', value: 'a\0b' }],
      [VARS_IPC.delete, { id: 'colour-match' }],
      [VARS_IPC.reveal, { id: 'colour-match', key: 'API_KEY', value: 'x' }],
      [VARS_IPC.values, 'colour-match']
    ];
    for (const [channel, payload] of bad) {
      await expect(h.call(channel, payload), `${channel} ${JSON.stringify(payload)}`).rejects.toThrow(channel);
    }
    expect(h.registry.refresh).not.toHaveBeenCalled();
    await expect(fs.readdir(path.join(h.root, 'env'))).rejects.toThrow();
  });

  it('refuses untrusted senders', async () => {
    const h = await harness({ trusted: false });
    await expect(h.call(VARS_IPC.values, { id: 'colour-match' })).rejects.toThrow('untrusted sender');
  });

  it('reports status without values, sets, re-resolves and delivers', async () => {
    const h = await harness();
    const before = await h.call<VarsStatus>(VARS_IPC.status, { id: 'colour-match' });
    expect(before.status).toBe('needs-setup');
    expect(before.keys.map((k) => [k.key, k.set, k.required, k.secret])).toEqual([['API_KEY', false, true, true], ['REGION', false, false, false]]);

    const after = await h.call<VarsStatus>(VARS_IPC.set, { id: 'colour-match', key: 'API_KEY', value: 'sk-abcdefghijkl1234' });
    expect(after.status).toBe('ok');
    expect(JSON.stringify(after)).not.toContain('sk-abcdefghijkl1234');
    expect(h.registry.refresh).toHaveBeenCalledWith(['colour-match']);
    expect(h.registry.emitChanged).toHaveBeenCalledWith({ ids: ['colour-match'], reason: 'reload' });

    expect(await h.call(VARS_IPC.values, { id: 'colour-match' })).toEqual({ API_KEY: 'sk-abcdefghijkl1234' });
    expect(await h.call(VARS_IPC.values, { id: 'builtin-one' })).toEqual({});
    await expect(h.call(VARS_IPC.set, { id: 'colour-match', key: 'UNDECLARED', value: 'x' })).rejects.toThrow(/not declared/);
    await expect(h.call(VARS_IPC.status, { id: 'builtin-one' })).rejects.toThrow(/no values/);

    const removed = await h.call<VarsStatus>(VARS_IPC.delete, { id: 'colour-match', key: 'API_KEY' });
    expect(removed.status).toBe('needs-setup');
  });

  it('reveal shows the value masked in a native dialog and returns nothing', async () => {
    const h = await harness({ response: 0 });
    await h.call(VARS_IPC.set, { id: 'colour-match', key: 'API_KEY', value: 'sk-abcdefghijkl1234' });
    const result = await h.call(VARS_IPC.reveal, { id: 'colour-match', key: 'API_KEY' });
    expect(result).toBeUndefined();
    const [, options] = h.showMessageBox.mock.calls[0] as unknown as [unknown, { message: string; detail: string; buttons: string[] }];
    expect(options.message).toBe('OpenAI API key');
    expect(options.detail).toBe('••••••••1234');
    expect(options.buttons).toEqual(['Copy', 'Done']);
    expect(h.writeClipboard).toHaveBeenCalledWith('sk-abcdefghijkl1234');
    await expect(h.call(VARS_IPC.reveal, { id: 'colour-match', key: 'REGION' })).rejects.toThrow(/no value/);
    expect(maskValue('short')).toBe('••••••••');
  });

  it('asks on remove and deletes values and provenance only when told to', async () => {
    const h = await harness();
    await h.call(VARS_IPC.set, { id: 'colour-match', key: 'REGION', value: 'eu' });
    const envKey = (await h.provenance.get('colour-match'))!.envKey;

    const keep = createRemoveValuesPrompt({ registry: h.registry, vars: h.vars, showMessageBox: async () => ({ response: 0 }), windowFor: () => null });
    expect(await keep(h.event as never, 'colour-match')).toBeUndefined();
    expect(await h.env.exists(envKey)).toBe(true);

    const show = vi.fn(async () => ({ response: 1 }));
    const remove = createRemoveValuesPrompt({ registry: h.registry, vars: h.vars, showMessageBox: show, windowFor: () => null });
    const after = await remove(h.event as never, 'colour-match');
    expect((show.mock.calls[0] as unknown as [unknown, { message: string; buttons: string[] }])[1]).toMatchObject({
      message: 'Also delete the values you entered for Colour match?',
      buttons: ['Keep', 'Delete']
    });
    expect(await h.env.exists(envKey)).toBe(true); // nothing is deleted before the folder is gone
    await after!();
    expect(await h.env.exists(envKey)).toBe(false);
    expect(await h.provenance.get('colour-match')).toBeNull();

    // No values file: no question.
    const silent = vi.fn(async () => ({ response: 1 }));
    const again = createRemoveValuesPrompt({ registry: h.registry, vars: h.vars, showMessageBox: silent, windowFor: () => null });
    expect(await again(h.event as never, 'colour-match')).toBeUndefined();
    expect(silent).not.toHaveBeenCalled();
  });
});
