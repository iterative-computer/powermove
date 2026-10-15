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

export interface Sender { id: number; once(event: 'destroyed', listener: () => void): unknown }

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

/** Bytes copied per `putProjectRange` call (the page reports progress between calls). */
export const PROJECT_RANGE_BYTES = 64 * 1024 * 1024;
/** Bytes per read of the open project file (`ProjectFiles.read` allows 1 MiB). */
const PROJECT_READ_BYTES = 1024 * 1024;

/** Reads a range of a project file the sender opened; rejects other senders' readers. */
export type ProjectRangeReader = (sender: Sender, token: string, offset: number, length: number) => Promise<Uint8Array>;

export function registerMediaStoreIpc(
  ipcMain: Pick<IpcMain, 'handle' | 'on'>,
  service: MediaStoreService,
  { isTrustedSender, projectRead }: { isTrustedSender(event: IpcMainInvokeEvent | IpcMainEvent): boolean; projectRead?: ProjectRangeReader }
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
    const expected: { blake3?: string; sha256?: string } = {};
    if (input['blake3'] !== undefined) expected.blake3 = String(input['blake3']);
    if (input['sha256'] !== undefined) expected.sha256 = String(input['sha256']);
    return service.store.finishStream(id, token(input['token']), expected);
  });
  /* Copies a range of the project file this window opened (Open…) straight
     into an upload, so embedded media reaches the store without passing
     through the page (section 21, "Embedded media moves to the media
     store"). The reader must belong to the same window. */
  ipcMain.handle(MEDIA_STORE_IPC.putProjectRange, async (event, value: unknown) => {
    const id = owner(event);
    const input = (value ?? {}) as Record<string, unknown>;
    const read = projectRead;
    if (!read) throw new Error('Project files are unavailable.');
    const readToken = input['readToken'];
    const offset = input['offset'];
    const length = input['length'];
    const at = input['at'];
    if (typeof readToken !== 'string' || !readToken || readToken.length > 100) throw new Error('Invalid project read token.');
    if (!Number.isSafeInteger(offset) || (offset as number) < 0 || !Number.isSafeInteger(length) || (length as number) < 1
      || (length as number) > PROJECT_RANGE_BYTES || !Number.isSafeInteger(at) || (at as number) < 0) throw new Error('Invalid project media range.');
    const upload = token(input['token']);
    for (let done = 0; done < (length as number);) {
      const want = Math.min(PROJECT_READ_BYTES, (length as number) - done);
      const data = await read(event.sender as unknown as Sender, readToken, (offset as number) + done, want);
      if (!(data instanceof Uint8Array) || data.byteLength < 1 || data.byteLength > want) throw new Error('The project media is truncated.');
      await service.store.chunk(id, upload, (at as number) + done, data, 'main');
      done += data.byteLength;
    }
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
  for (const [channel, kind] of [[MEDIA_STORE_IPC.convertStill, 'still'], [MEDIA_STORE_IPC.convertVideo, 'video']] as const) {
    ipcMain.handle(channel, (event, value: unknown) => {
      owner(event);
      const input = (value ?? {}) as Record<string, unknown>;
      const name = typeof input['name'] === 'string' ? input['name'].slice(0, 1000) : '';
      const convert = (request: Parameters<MediaStoreService['convertStill']>[0]) => kind === 'still' ? service.convertStill(request) : service.convertVideo(request);
      if (input['link'] !== undefined) {
        if (!validLink(input['link'])) throw new Error('Invalid media link.');
        return convert({ link: input['link'], name });
      }
      return convert({ hash: String(input['hash'] ?? ''), name });
    });
  }
  ipcMain.handle(MEDIA_STORE_IPC.convertUpload, (event, value: unknown) => {
    const id = owner(event);
    const input = (value ?? {}) as Record<string, unknown>;
    const kind = input['kind'] === 'still' || input['kind'] === 'video' ? input['kind'] : null;
    if (!kind) throw new Error('Invalid conversion.');
    const name = typeof input['name'] === 'string' ? input['name'].slice(0, 1000) : '';
    return service.convertUpload(id, token(input['token']), { kind, name, blake3: String(input['blake3'] ?? '') });
  });
  ipcMain.handle(MEDIA_STORE_IPC.removeKey, (event, value: unknown) => { owner(event); return service.store.removeKey(key(value) ?? ''); });
}
