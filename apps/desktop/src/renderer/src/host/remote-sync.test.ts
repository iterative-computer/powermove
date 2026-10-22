import { expect, it } from 'vitest';
import { attachRemoteSync, type SyncLink } from './remote-sync';
import { WEB } from '../../../shared/wire';

it('pulls the latest host document when boot restored an empty document after joining', async () => {
  const copy = <T>(value: T): T => structuredClone(value);
  let host = { id: 'Pone', revision: 2, layers: [{ id: 'Lone' }] };
  const patches: unknown[] = [], leaves: unknown[] = [];
  const listeners = new Map<string, Set<(...args: any[]) => void>>();
  const PM: any = {
    proj: { id: 'Pone', revision: 0, layers: [] },
    bus: {
      on(event: string, callback: (...args: any[]) => void) { const set = listeners.get(event) ?? new Set(); set.add(callback); listeners.set(event, set); return () => { set.delete(callback); }; },
      emit(event: string) { for (const callback of listeners.get(event) ?? []) callback(); }
    },
    replaceProject(doc: unknown) { PM.proj = copy(doc); PM.bus.emit('project'); }
  };
  const link: SyncLink = {
    async invoke<T>(channel: string, ...args: unknown[]): Promise<T> {
      if (channel === WEB.syncJoin) return { seq: host.revision, doc: copy(host) } as T;
      if (channel === WEB.syncPatch) { patches.push(args); return host.revision as T; }
      throw new Error(channel);
    },
    send: (channel, ...args) => { if (channel === WEB.syncLeave) leaves.push(args); },
    on: () => () => {}
  };
  const dispose = attachRemoteSync(link, PM);
  try {
    await PM.RemoteSync.flush();
    expect(PM.proj).toEqual(host);
    // The asynchronous editor boot subsequently replaced the same project ID.
    PM.proj = { id: 'Pone', revision: 0, layers: [] };
    // Another host edit also arrived after the tab obtained its initial seed.
    host = { ...host, revision: 3, layers: [...host.layers, { id: 'Ltwo' }] };
    await PM.RemoteSync.refresh(); await PM.RemoteSync.flush();
    expect(PM.proj).toEqual(host);
    expect(patches).toEqual([]);
    expect(leaves).toEqual([['Pone']]);
  } finally { dispose(); }
});

it('holds native DOM boot writes until extensions settle and the worker document is pulled again', async () => {
  const host = { id: 'Pone', revision: 2, layers: [{ id: 'Lone', fx: [{ id: 'Fone', type: 'user.effect', on: true, p: {} }] }] };
  const patches: unknown[] = [];
  const callbacks = new Map<string, () => void>();
  const PM: any = {
    proj: structuredClone(host),
    bus: { on: (event: string, fn: () => void) => { callbacks.set(event, fn); return () => callbacks.delete(event); }, emit: (event: string) => callbacks.get(event)?.() },
    replaceProject: (doc: unknown) => { PM.proj = structuredClone(doc); PM.bus.emit('project'); }
  };
  const link: SyncLink = {
    invoke: async <T>(channel: string, ...args: unknown[]) => {
      if (channel === WEB.syncJoin) return { seq: 2, doc: structuredClone(host) } as T;
      if (channel === WEB.syncPatch) { patches.push(args); return 3 as T; }
      throw new Error(channel);
    }, send: () => {}, on: () => () => {}
  };
  const dispose = attachRemoteSync(link, PM, { deferWrites: true });
  try {
    await PM.RemoteSync.flush();
    PM.proj.layers[0].fx[0].missing = true; PM.bus.emit('layers');
    await PM.RemoteSync.flush();
    expect(patches).toEqual([]);
    await PM.RemoteSync.ready();
    expect(PM.proj).toEqual(host);
    PM.proj.layers[0].fx[0].on = false; PM.bus.emit('layers');
    await PM.RemoteSync.flush();
    expect(patches).toHaveLength(1);
  } finally { dispose(); }
});
