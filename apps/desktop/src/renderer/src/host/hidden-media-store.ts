/** Native hidden tabs use the same durable host store as desktop windows.
 * The authenticated web link replaces preload IPC; successful puts mean the
 * host has verified the bytes, rather than an IndexedDB mirror still uploading. */
import { MEDIA_STORE_IPC, type MediaStoreBridge } from '../../../shared/media-store';
import type { MediaLink } from './remote-media';

export function hiddenMediaStore(link: MediaLink): MediaStoreBridge {
  const call = <T>(channel: string, ...args: unknown[]) => link.invoke<T>(channel, ...args);
  return {
    configSync: () => ({ enabled: true }),
    record: key => call(MEDIA_STORE_IPC.record, key),
    classify: path => call(MEDIA_STORE_IPC.classify, path),
    putFile: request => call(MEDIA_STORE_IPC.putFile, request),
    beginPut: request => call(MEDIA_STORE_IPC.beginPut, request),
    putChunk: (token, offset, data) => call(MEDIA_STORE_IPC.putChunk, { token, offset, data }),
    finishPut: (token, expected) => call(MEDIA_STORE_IPC.finishPut, { token, ...expected }),
    putProjectRange: (token, readToken, offset, length, at) => call(MEDIA_STORE_IPC.putProjectRange, { token, readToken, offset, length, at }),
    abortPut: token => call(MEDIA_STORE_IPC.abortPut, token),
    link: path => call(MEDIA_STORE_IPC.link, path),
    open: request => call(MEDIA_STORE_IPC.open, request),
    read: (token, offset, length) => call(MEDIA_STORE_IPC.read, { token, offset, length }),
    close: token => call(MEDIA_STORE_IPC.close, token),
    relink: request => call(MEDIA_STORE_IPC.relink, request),
    bind: (projectId, assets) => call(MEDIA_STORE_IPC.bind, { projectId, assets }),
    removeKey: key => call(MEDIA_STORE_IPC.removeKey, key),
    convertStill: request => call(MEDIA_STORE_IPC.convertStill, request),
    convertVideo: request => call(MEDIA_STORE_IPC.convertVideo, request),
    convertUpload: (token, request) => call(MEDIA_STORE_IPC.convertUpload, { token, ...request })
  };
}
