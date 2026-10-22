import { digestInstructions, type DigestInstruction } from '../../shared/document-digest';
import { canonicalStringify } from '../../shared/canonical-json';
import { createHash, randomUUID, type Hash } from 'node:crypto';
import { mkdtemp, mkdir, open, rm, type FileHandle } from 'node:fs/promises';
import path from 'node:path';
import type { IpcMain, IpcMainEvent, IpcMainInvokeEvent } from 'electron';
import { DOCUMENT_KERNEL_IPC, type DocumentKernelReceipt } from '../../shared/document-kernel';

type Sender = IpcMainEvent | IpcMainInvokeEvent;
type Stream = { owner: number; generation: number; dir: string; file: FileHandle; hash: Hash; bytes: number; closed: boolean; cancelled: boolean; pendingWrites: number; queuedBytes: number; tail: Promise<void>; idle?: NodeJS.Timeout; cleanup?: Promise<void> };
type Digest = { owner: number; hash: Hash; bytes: number; idle?: NodeJS.Timeout };
type Owner = { generation: number; retiring?: Promise<void>; beginning: Set<Promise<string>>; commits: Set<Promise<DocumentKernelReceipt>>; unwatch: () => void };
const LIMIT = 512 * 1024 * 1024;
const CHUNK_LIMIT = 1024 * 1024;
const QUEUED_WRITES = 2;
const BATCH_LIMIT = 256 * 1024;
const IDLE_MS = 120_000;
const BUSY_RETRY_MS = 250;
const CALLS = new Set(['refresh', 'pollRefresh', 'checkpoint', 'cancel', 'activate', 'resync', 'apply', 'revision', 'hash', 'consistencyHash', 'toJson', 'intern', 'release', 'ev', 'evBatch', 'expressionError', 'query', 'define', 'addFontAnswers', 'addFont', 'setFontSeeds', 'clearFonts', 'fontOutlines', 'fontReport', 'stats', 'heap']);

/** Files and evaluator owners never cross the page's trust boundary. */
export function installDocumentKernelIpc(options: {
  ipc: IpcMain;
  cache: string;
  trusted(event: Sender): boolean;
  call(owner: number, request: string, bytes?: Uint8Array): Promise<string>;
  /** After a large document is ingested for this window (memory-relief.ts). */
  ingested?(sender: Sender['sender'], bytes: number): void;
}): () => Promise<void> {
  const streams = new Map<string, Stream>();
  const owners = new Map<number, Owner>();
  const beginning = new Set<number>();
  const beginningJobs = new Set<Promise<string>>();
  const digests = new Map<string, Digest>();
  let disposed = false;
  const expireStream = (token: string, s: Stream): void => {
    clearTimeout(s.idle);
    if (s.closed || s.pendingWrites) return;
    s.idle = setTimeout(() => {
      if (!s.closed && !s.pendingWrites) void cleanup(token, s).catch(error => console.warn('[engine-doc] idle document cleanup failed', error));
    }, IDLE_MS);
    s.idle.unref();
  };
  const expireDigest = (token: string, d: Digest): void => {
    clearTimeout(d.idle);
    d.idle = setTimeout(() => { if (digests.get(token) === d) digests.delete(token); }, IDLE_MS);
    d.idle.unref();
  };
  const cleanup = (token: string, s: Stream): Promise<void> => {
    if (s.cleanup) return s.cleanup;
    s.closed = s.cancelled = true;
    clearTimeout(s.idle);
    s.cleanup = (async () => {
      try {
        await s.tail.catch(() => {});
        await s.file.close().catch(() => {});
        await rm(s.dir, { recursive: true, force: true });
      } finally {
        // Keep the owner reservation until pending writes and removal finish.
        if (streams.get(token) === s) streams.delete(token);
      }
    })();
    return s.cleanup;
  };
  const retire = (owner: number, state: Owner): void => {
    state.generation++;
    for (const [token, d] of digests) if (d.owner === owner) { clearTimeout(d.idle); digests.delete(token); }
    const cleaning = [...streams].filter(([, s]) => s.owner === owner).map(([token, s]) => cleanup(token, s));
    const pending = [...state.beginning, ...state.commits, ...cleaning];
    const previous = state.retiring;
    const retiring = (async () => {
      if (previous) await previous.catch(() => {});
      await Promise.allSettled(pending);
      if (!disposed) await options.call(owner, '{"op":"close"}');
    })();
    state.retiring = retiring;
    void retiring.catch(error => console.warn('[engine-doc] document owner cleanup failed', error)).finally(() => {
      if (state.retiring === retiring) state.retiring = undefined;
    });
  };
  const ownerOf = (event: Sender): number => {
    if (disposed || !options.trusted(event) || event.sender.isDestroyed()) throw new Error('Unauthorized document owner');
    const owner = event.sender.id;
    if (!owners.has(owner)) {
      const sender = event.sender;
      const state: Owner = { generation: 0, beginning: new Set(), commits: new Set(), unwatch: () => {} };
      const navigated = (details: { isMainFrame: boolean; isSameDocument: boolean }) => {
        if (details.isMainFrame && !details.isSameDocument) retire(owner, state);
      };
      const gone = () => retire(owner, state);
      const destroyed = () => {
        retire(owner, state);
        void state.retiring?.catch(() => {}).then(() => {
          state.unwatch();
          if (owners.get(owner) === state) owners.delete(owner);
        });
      };
      sender.on('did-start-navigation', navigated);
      sender.on('render-process-gone', gone);
      sender.once('destroyed', destroyed);
      state.unwatch = () => {
        sender.off('did-start-navigation', navigated);
        sender.off('render-process-gone', gone);
        sender.off('destroyed', destroyed);
      };
      owners.set(owner, state);
    }
    return owner;
  };
  const owned = (event: Sender, token: unknown): [string, Stream] => {
    const owner = ownerOf(event);
    const s = typeof token === 'string' ? streams.get(token) : undefined;
    if (!s || s.owner !== owner || s.generation !== owners.get(owner)?.generation || s.closed || owners.get(owner)?.retiring) throw new Error('The document stream is closed');
    return [token as string, s];
  };
  options.ipc.on(DOCUMENT_KERNEL_IPC.call, (event, request: unknown, bytes?: unknown) => {
    let pending: Promise<string>;
    try {
      const owner = ownerOf(event);
      if (owners.get(owner)?.retiring) throw Object.assign(new Error('The document owner is restarting'), { code: 'unavailable' });
      if (typeof request !== 'string' || request.length > LIMIT) throw new Error('Invalid document request');
      const parsed = JSON.parse(request);
      if (!parsed || !CALLS.has(parsed.op)) throw new Error('Invalid document operation');
      if (bytes !== undefined && (!(bytes instanceof Uint8Array) || bytes.byteLength > LIMIT || !['apply', 'addFont'].includes(parsed.op))) throw new Error('Invalid document binary payload');
      pending = options.call(owner, request, bytes as Uint8Array | undefined);
    } catch (error) { pending = Promise.resolve(JSON.stringify({ ok: false, code: (error as { code?: string }).code || 'invalid', error: String(error) })); }
    void pending.catch(error => JSON.stringify({ ok: false, code: 'unavailable', error: String(error) })).then(answer => {
      if (!event.sender.isDestroyed()) event.returnValue = answer;
    });
  });
  options.ipc.handle(DOCUMENT_KERNEL_IPC.begin, async event => {
    const owner = ownerOf(event);
    const state = owners.get(owner)!;
    const generation = state.generation;
    if (state.retiring) await state.retiring.catch(() => {});
    if (disposed || event.sender.isDestroyed() || generation !== state.generation) throw new Error('The document owner closed');
    if (beginning.has(owner) || [...streams.values()].some(s => s.owner === owner)) throw new Error('A document stream is already open');
    if (streams.size + beginningJobs.size >= 32) throw new Error('Too many document streams');
    beginning.add(owner);
    const job = (async () => {
      let dir: string | undefined;
      try {
        await mkdir(options.cache, { recursive: true });
        dir = await mkdtemp(path.join(options.cache, 'document-'));
        const file = await open(path.join(dir, 'canonical.json'), 'wx', 0o600);
        if (disposed || event.sender.isDestroyed() || generation !== state.generation) { await file.close(); throw new Error('The document owner closed'); }
        const token = randomUUID();
        const stream: Stream = { owner, generation, dir, file, hash: createHash('sha256'), bytes: 0, closed: false, cancelled: false, pendingWrites: 0, queuedBytes: 0, tail: Promise.resolve() };
        streams.set(token, stream);
        expireStream(token, stream);
        return token;
      } catch (error) { if (dir) await rm(dir, { recursive: true, force: true }); throw error; }
      finally { beginning.delete(owner); }
    })();
    beginningJobs.add(job);
    state.beginning.add(job);
    try { return await job; } finally { beginningJobs.delete(job); state.beginning.delete(job); }
  });
  options.ipc.handle(DOCUMENT_KERNEL_IPC.write, async (event, token: unknown, bytes: unknown) => {
    const [, s] = owned(event, token);
    if (!(bytes instanceof Uint8Array) || bytes.byteLength === 0 || bytes.byteLength > CHUNK_LIMIT || s.bytes + bytes.byteLength > LIMIT) throw new Error('Invalid document chunk');
    if (s.pendingWrites >= QUEUED_WRITES || s.queuedBytes + bytes.byteLength > QUEUED_WRITES * CHUNK_LIMIT) throw new Error('Document stream backpressure: await pending writes');
    clearTimeout(s.idle);
    s.bytes += bytes.byteLength; s.pendingWrites++; s.queuedBytes += bytes.byteLength;
    const job = s.tail.then(async () => {
      let offset = 0;
      while (offset < bytes.byteLength) {
        if (s.cancelled) throw new Error('The document stream is closed');
        const written = await s.file.write(bytes, offset, bytes.byteLength - offset);
        if (!written.bytesWritten) throw new Error('The document cache did not accept data');
        offset += written.bytesWritten;
      }
      if (s.cancelled) throw new Error('The document stream is closed');
      s.hash.update(bytes);
    });
    s.tail = job.finally(() => { s.pendingWrites--; s.queuedBytes -= bytes.byteLength; expireStream(token as string, s); });
    await s.tail;
  });
  options.ipc.handle(DOCUMENT_KERNEL_IPC.commit, async (event, token: unknown, document: unknown, revision: unknown): Promise<DocumentKernelReceipt> => {
    const [key, s] = owned(event, token);
    if (typeof document !== 'string' || !document || document.length > 256 || !Number.isSafeInteger(revision) || (revision as number) < 0) throw new Error('Invalid document revision');
    s.closed = true;
    clearTimeout(s.idle);
    const state = owners.get(s.owner)!;
    const current = () => !disposed && !s.cancelled && !event.sender.isDestroyed() && state.generation === s.generation;
    const job = (async (): Promise<DocumentKernelReceipt> => {
      try {
        if (!current()) throw new Error('The document owner closed');
        await s.tail;
        if (!current()) throw new Error('The document owner closed');
        await s.file.close();
        const hash = s.hash.digest('hex');
        const request = async (op: string, args?: object): Promise<any> => {
          if (!current()) throw new Error('The document owner closed');
          const reply = JSON.parse(await options.call(s.owner, JSON.stringify({ op, args })));
          if (!current()) throw new Error('The document owner closed');
          if (!reply.ok) throw Object.assign(new Error(reply.error || 'Document ingestion failed'), { code: reply.code });
          return reply.value;
        };
        const deadline = Date.now() + 120_000;
        let result: any;
        while (true) {
          try {
            // `hash` is this stream's SHA-256; the utility checks the file
            // against it and answers the canonical hash of the tree it parsed
            // (the stream may be JSON text in any member order).
            result = await request('load', { path: path.join(s.dir, 'canonical.json'), document, revision, hash, source: 'text' });
            break;
          } catch (error) {
            if ((error as { code?: string }).code !== 'busy') throw error;
            const remaining = deadline - Date.now();
            if (remaining <= 0) throw new Error('Document ingestion timed out');
            await new Promise(resolve => setTimeout(resolve, Math.min(BUSY_RETRY_MS, remaining)));
            if (!current()) throw new Error('The document owner closed');
          }
        }
        if (Date.now() > deadline) throw new Error('Document ingestion timed out');
        while (result.pending) {
          const remaining = deadline - Date.now();
          if (remaining <= 0) throw new Error('Document ingestion timed out');
          await new Promise(resolve => setTimeout(resolve, Math.min(10, remaining)));
          result = await request('poll');
          if (Date.now() > deadline) throw new Error('Document ingestion timed out');
        }
        if (result.revision !== revision || typeof result.hash !== 'string' || !/^[0-9a-f]{64}$/.test(result.hash)) throw new Error('Document ingestion returned a different revision');
        try { options.ingested?.(event.sender, s.bytes); } catch { /* an optimization only */ }
        return result;
      } catch (error) {
        // A previous page must not cancel the new page's prepared document.
        if (current()) await Promise.resolve().then(() => options.call(s.owner, '{"op":"cancel"}')).catch(() => {});
        throw error;
      } finally { await cleanup(key, s); }
    })();
    state.commits.add(job);
    try { return await job; } finally { state.commits.delete(job); }
  });
  options.ipc.handle(DOCUMENT_KERNEL_IPC.abort, async (event, token: unknown) => {
    const owner = ownerOf(event);
    if (typeof token === 'string' && digests.get(token)?.owner === owner) { clearTimeout(digests.get(token)!.idle); digests.delete(token); return; }
    const [key, s] = owned(event, token);
    await cleanup(key, s);
  });
  options.ipc.handle(DOCUMENT_KERNEL_IPC.digestBegin, (event, prefix: unknown) => {
    const owner = ownerOf(event);
    if (owners.get(owner)?.retiring) throw Object.assign(new Error('The document owner is restarting'), { code: 'unavailable' });
    if (typeof prefix !== 'string' || prefix.length > 16 || digests.size >= 128 || [...digests.values()].filter(d => d.owner === owner).length >= 8) throw new Error('Invalid digest stream');
    const token = randomUUID();
    const digest: Digest = { owner, hash: createHash('sha256').update(prefix, 'utf8'), bytes: 0 };
    digests.set(token, digest);
    expireDigest(token, digest);
    return token;
  });
  options.ipc.handle(DOCUMENT_KERNEL_IPC.digestWrite, (event, token: unknown, chunk: unknown) => {
    const owner = ownerOf(event);
    const d = typeof token === 'string' ? digests.get(token) : undefined;
    if (!d || d.owner !== owner || !(chunk instanceof Uint8Array) || chunk.byteLength > 1024 * 1024 || d.bytes + chunk.byteLength > LIMIT) throw new Error('Invalid digest chunk');
    d.bytes += chunk.byteLength; d.hash.update(chunk); expireDigest(token as string, d);
  });
  options.ipc.handle(DOCUMENT_KERNEL_IPC.digestEnd, (event, token: unknown) => {
    const owner = ownerOf(event);
    const d = typeof token === 'string' ? digests.get(token) : undefined;
    if (!d || d.owner !== owner) throw new Error('The digest stream is closed');
    clearTimeout(d.idle);
    digests.delete(token as string);
    return d.hash.digest('hex');
  });
  options.ipc.handle(DOCUMENT_KERNEL_IPC.digestBatch, (event, instructions: unknown) => {
    ownerOf(event);
    if (!Array.isArray(instructions) || instructions.length > 128) throw new Error('Invalid digest batch');
    // Bound both the received JSON and expansion of numeric references into
    // 64-character hashes before a canonical writer runs on main.
    let bytes = 0, nodes = 0;
    const add = (count: number) => { bytes += count; if (bytes > BATCH_LIMIT) throw new Error('Digest batch is too large'); };
    const quote = (value: string): void => {
      add(2);
      for (let i = 0; i < value.length; i++) {
        const code = value.charCodeAt(i);
        if (code === 34 || code === 92 || code === 8 || code === 9 || code === 10 || code === 12 || code === 13) add(2);
        else if (code < 32) add(6);
        else if (code < 128) add(1);
        else if (code < 2048) add(2);
        else if (code >= 0xd800 && code <= 0xdbff && value.charCodeAt(i + 1) >= 0xdc00 && value.charCodeAt(i + 1) <= 0xdfff) { add(4); i++; }
        else add(code >= 0xd800 && code <= 0xdfff ? 6 : 3);
      }
    };
    const visit = (value: any, depth: number): void => {
      if (++nodes > 131072 || depth > 64) throw new Error('Digest batch is too large');
      if (typeof value === 'string') quote(value);
      else if (Array.isArray(value)) {
        if (value.length > 32768) throw new Error('Digest batch is too large');
        add(2);
        for (let i = 0; i < value.length; i++) { if (i) add(1); visit(value[i], depth + 1); }
      } else if (value && typeof value === 'object') {
        const prototype = Object.getPrototypeOf(value);
        if (prototype !== Object.prototype && prototype !== null) throw new Error('A digest batch needs plain JSON values');
        add(2);
        let first = true;
        for (const key in value) if (Object.hasOwn(value, key)) {
          if (!first) add(1); first = false;
          quote(key); add(1); visit(value[key], depth + 1);
        }
      } else add(typeof value === 'number' ? JSON.stringify(Number.isFinite(value) ? value : null).length : typeof value === 'boolean' ? value ? 4 : 5 : 4);
    };
    visit(instructions, 0);
    for (const instruction of instructions) {
      const refs = instruction && typeof instruction === 'object'
        ? Array.isArray(instruction.array) ? instruction.array
          : Array.isArray(instruction.object) ? instruction.object.map((entry: unknown) => Array.isArray(entry) ? entry[1] : undefined) : []
        : [];
      for (const ref of refs) if (typeof ref === 'number') add(66 - JSON.stringify(Number.isFinite(ref) ? ref : null).length);
    }
    return digestInstructions(instructions as DigestInstruction[], (value, prefix = '') => {
      const json = canonicalStringify(value);
      if (json === undefined) throw new Error('A digest needs a JSON value');
      return createHash('sha256').update(prefix, 'utf8').update(json, 'utf8').digest('hex');
    });
  });
  return async () => {
    disposed = true;
    for (const state of owners.values()) state.unwatch();
    const retirements = [...owners.values()].map(state => state.retiring).filter((job): job is Promise<void> => !!job);
    owners.clear();
    for (const digest of digests.values()) clearTimeout(digest.idle);
    digests.clear();
    options.ipc.removeAllListeners(DOCUMENT_KERNEL_IPC.call);
    for (const channel of [DOCUMENT_KERNEL_IPC.begin, DOCUMENT_KERNEL_IPC.write, DOCUMENT_KERNEL_IPC.commit, DOCUMENT_KERNEL_IPC.abort, DOCUMENT_KERNEL_IPC.digestBegin, DOCUMENT_KERNEL_IPC.digestWrite, DOCUMENT_KERNEL_IPC.digestEnd, DOCUMENT_KERNEL_IPC.digestBatch]) options.ipc.removeHandler(channel);
    await Promise.allSettled([...beginningJobs]);
    await Promise.all([...streams].map(([token, s]) => cleanup(token, s)));
    await Promise.allSettled(retirements);
  };
}
