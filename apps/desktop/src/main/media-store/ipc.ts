/*
 * The media store's IPC (preload `mediaStore`). Every handler checks that
 * the sender is an app window's main frame (never an extension sandbox
 * frame); reads, uploads, and bindings belong to the window that made them
 * and go away with it.
 */
import path from 'node:path';

import type { IpcMain, IpcMainEvent, IpcMainInvokeEvent } from 'electron';

import { MEDIA_STORE_IPC, validStorageKey, type MediaRecordSource, type MediaStoreConfig } from '../../shared/media-store';
import { MAX_LINK_PATH, type MediaStoreService, validLink } from './service';

const ORIGINS = new Set<MediaRecordSource>(['import', 'stream', 'indexeddb', 'project', 'rendition']);
const PROJECT = /^[A-Za-z0-9_-]{1,120}$/;

interface Sender { id: number; once(event: 'destroyed', listener: () => void): unknown }

const filePath = (value: unknown): string => {
  if (typeof value !== 'string' || !path.isAbsolute(value) || value.length > MAX_LINK_PATH || value.includes('\0')) throw new Error('A media file path must be absolute.');
  return value;
};
const key = (value: unknown): string | null => {
  if (value === null) return null;
  if (!validStorageKey(value)) throw new Error('Invalid media storage key.');
  return value;
};
const token = (value: unknown): string => {
  if (typeof value !== 'string' || !/^[a-f0-9]{32}$/.test(value)) throw new Error('Invalid media token.');
  return value;
};
const revision = (value: unknown): string | null => {
  if (value === undefined || value === null) return null;
  if (typeof value !== 'string' || !value || value.length > 200) throw new Error('Invalid media revision.');
  return value;
};

export function registerMediaStoreIpc(
  ipcMain: Pick<IpcMain, 'handle' | 'on'>,
  service: MediaStoreService,
  { isTrustedSender }: { isTrustedSender(event: IpcMainInvokeEvent | IpcMainEvent): boolean }
): void {
  const watched = new WeakSet<object>();
  const owner = (event: IpcMainInvokeEvent | IpcMainEvent): number => {
    if (!isTrustedSender(event)) throw new Error('Unauthorized IPC sender');
    const sender = event.sender as unknown as Sender;
    if (!watched.has(sender)) {
      watched.add(sender);
      sender.once('destroyed', () => { void service.release(sender.id).catch(() => undefined); });
    }
    return sender.id;
  };

  ipcMain.on(MEDIA_STORE_IPC.config, (event) => {
    try { owner(event); event.returnValue = { enabled: service.enabled } satisfies MediaStoreConfig; }
    catch { event.returnValue = { enabled: false } satisfies MediaStoreConfig; }
  });
  ipcMain.handle(MEDIA_STORE_IPC.record, (event, value: unknown) => { owner(event); return service.store.record(key(value) ?? ''); });
  ipcMain.handle(MEDIA_STORE_IPC.classify, (event, value: unknown) => { owner(event); return service.classify(filePath(value)); });
  ipcMain.handle(MEDIA_STORE_IPC.putFile, async (event, value: unknown) => {
    owner(event);
    const input = (value ?? {}) as Record<string, unknown>;
    const type = typeof input['type'] === 'string' ? String(input['type']).slice(0, 200) : '';
    const { identity: _identity, ...record } = await service.store.putFile(filePath(input['path']), { key: key(input['key'] ?? null), type, revision: revision(input['revision']), origin: 'import' });
    return record;
  });
  ipcMain.handle(MEDIA_STORE_IPC.beginPut, (event, value: unknown) => {
    const id = owner(event);
    const input = (value ?? {}) as Record<string, unknown>;
    const origin = ORIGINS.has(input['origin'] as MediaRecordSource) ? input['origin'] as MediaRecordSource : 'stream';
    return service.store.beginStream(id, {
      key: key(input['key'] ?? null), type: typeof input['type'] === 'string' ? input['type'] : '',
      size: input['size'] as number, revision: revision(input['revision']), origin
    });
  });
  ipcMain.handle(MEDIA_STORE_IPC.putChunk, (event, value: unknown) => {
    const id = owner(event);
    const input = (value ?? {}) as Record<string, unknown>;
    if (!(input['data'] instanceof Uint8Array) || !Number.isSafeInteger(input['offset'])) throw new Error('Invalid media upload chunk.');
    return service.store.chunk(id, token(input['token']), input['offset'] as number, input['data']);
  });
  ipcMain.handle(MEDIA_STORE_IPC.finishPut, (event, value: unknown) => {
    const id = owner(event);
    const input = (value ?? {}) as Record<string, unknown>;
    return service.store.finishStream(id, token(input['token']), { blake3: String(input['blake3'] ?? '') });
  });
  ipcMain.handle(MEDIA_STORE_IPC.abortPut, (event, value: unknown) => service.store.abortStream(owner(event), token(value)));
  ipcMain.handle(MEDIA_STORE_IPC.link, (event, value: unknown) => { owner(event); return service.link(filePath(value)); });
  ipcMain.handle(MEDIA_STORE_IPC.open, (event, value: unknown) => {
    const id = owner(event);
    const input = (value ?? {}) as Record<string, unknown>;
    const roots = Array.isArray(input['searchRoots']) ? (input['searchRoots'] as unknown[]).slice(0, 8).map(filePath) : [];
    if (input['link'] !== undefined) {
      if (!validLink(input['link'])) throw new Error('Invalid media link.');
      return service.open(id, { link: input['link'], searchRoots: roots });
    }
    if (input['key'] !== undefined) return service.open(id, { key: key(input['key']) ?? '' });
    return service.open(id, { hash: String(input['hash'] ?? '') });
  });
  ipcMain.handle(MEDIA_STORE_IPC.read, (event, value: unknown) => {
    const id = owner(event);
    const input = (value ?? {}) as Record<string, unknown>;
    return service.read(id, token(input['token']), input['offset'] as number, input['length'] as number);
  });
  ipcMain.handle(MEDIA_STORE_IPC.close, (event, value: unknown) => service.close(owner(event), token(value)));
  ipcMain.handle(MEDIA_STORE_IPC.relink, (event, value: unknown) => {
    owner(event);
    const input = (value ?? {}) as Record<string, unknown>;
    if (!validLink(input['link'])) throw new Error('Invalid media link.');
    const roots = Array.isArray(input['searchRoots']) ? (input['searchRoots'] as unknown[]).slice(0, 8).map(filePath) : [];
    return service.relink(input['link'], roots);
  });
  ipcMain.handle(MEDIA_STORE_IPC.bind, (event, value: unknown) => {
    const id = owner(event);
    const input = (value ?? {}) as Record<string, unknown>;
    if (typeof input['projectId'] !== 'string' || !PROJECT.test(input['projectId']) || !input['assets'] || typeof input['assets'] !== 'object') throw new Error('Invalid media binding.');
    service.bind(id, input['projectId'], input['assets'] as Record<string, never>);
  });
  ipcMain.handle(MEDIA_STORE_IPC.removeKey, (event, value: unknown) => { owner(event); return service.store.removeKey(key(value) ?? ''); });
}
