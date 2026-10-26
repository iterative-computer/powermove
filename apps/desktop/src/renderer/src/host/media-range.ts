import type { MediaReadSource } from '../../../shared/media-source';
import { LIBRARY_MEDIA_CHUNK, type LibraryMediaRecord } from '../../../shared/library-media';

/** Small independent responses avoid Chromium's whole-response Blob creation
 * for custom protocols. Reject ignored ranges before consuming their bodies. */
export function mediaRangeSource(record: LibraryMediaRecord): MediaReadSource {
  return { size: record.size, type: record.type, sha256: record.sha256,
    async read(offset, length, signal) {
      signal?.throwIfAborted();
      if (!Number.isSafeInteger(offset) || offset < 0 || !Number.isSafeInteger(length) || length < 1
        || length > LIBRARY_MEDIA_CHUNK || offset > record.size - length) throw new Error('Invalid media range.');
      const end = offset + length - 1;
      const response = await fetch(record.url, { cache: 'no-store', signal, headers: { Range: `bytes=${offset}-${end}` } });
      if (response.status !== 206 || response.headers.get('Content-Range') !== `bytes ${offset}-${end}/${record.size}`
        || response.headers.get('Content-Length') !== String(length) || !response.body) {
        await response.body?.cancel().catch(() => undefined);
        throw new Error(`The library did not return the requested media range (HTTP ${response.status}).`);
      }
      const reader = response.body.getReader(), bytes = new Uint8Array(length);
      const cancel = () => { void reader.cancel(signal?.reason).catch(() => undefined); };
      signal?.addEventListener('abort', cancel, { once: true });
      let received = 0, complete = false;
      try {
        for (;;) {
          signal?.throwIfAborted();
          const result = await reader.read();
          if (result.done) { complete = true; break; }
          if (result.value.length > length - received) throw new Error('The library returned too much media data.');
          bytes.set(result.value, received); received += result.value.length;
        }
        signal?.throwIfAborted();
        if (received !== length) throw new Error('The project media is incomplete.');
        return bytes;
      } finally {
        signal?.removeEventListener('abort', cancel);
        if (!complete) await reader.cancel().catch(() => undefined);
        reader.releaseLock();
      }
    }
  };
}
