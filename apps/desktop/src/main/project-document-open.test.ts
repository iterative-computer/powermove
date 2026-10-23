import { EventEmitter } from 'node:events';
import { createHash } from 'node:crypto';
import { mkdtemp, open, readFile, rm, writeFile, type FileHandle } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { Worker } from 'node:worker_threads';
import { afterEach, beforeEach, expect, it, vi, type MockInstance } from 'vitest';

const tracked = vi.hoisted(() => ({ path: '', handles: [] as FileHandle[] }));
vi.mock('node:fs/promises', async importOriginal => {
  const fs = await importOriginal<typeof import('node:fs/promises')>();
  return { ...fs, open: async (...args: Parameters<typeof fs.open>) => {
    const handle = await fs.open(...args);
    if (args[0] === tracked.path && args[1] === 'r') tracked.handles.push(handle);
    return handle;
  } };
});
vi.mock('electron', () => ({ BrowserWindow: { fromWebContents: vi.fn() }, dialog: {} }));

import { ProjectFiles } from './project-files';
import * as validator from './project-document-validator';
import * as projectHash from './project-file-hash';
import { openProjectForWindow } from './save';
import { FILE_CHUNK_BYTES } from './file-upload';
import { projectFooter, type ProjectRecord } from '../shared/project-incremental';
import { encodeProjectContainer } from '../shared/project-container';

const actualValidate = validator.validateProjectDocumentInWorker, actualHash = projectHash.hashProjectFile;
const digest = (bytes: Uint8Array) => createHash('sha256').update(bytes).digest('hex');
const value = { proj: { id: 'embedded', name: 'complete project', w: 100, h: 200, layers: [], unknown: { keep: true } },
  history: { entries: [{ label: 'Move', value: { x: 2 } }], index: 0 }, ws: { panels: ['timeline'] } };
const source = JSON.stringify(value);
let directory: string, registry: string, files: ProjectFiles;
const workers: Worker[] = [], buffers: Buffer[] = [];
let validate: MockInstance<typeof validator.validateProjectDocumentInWorker>;
function spawn(script: string, options: ConstructorParameters<typeof Worker>[1]) {
  const worker = new Worker(script, options); workers.push(worker); return worker;
}
beforeEach(async () => {
  directory = await mkdtemp(path.join(tmpdir(), 'powermove-project-worker-'));
  tracked.path = path.join(directory, 'Project.pmv'); tracked.handles.length = 0;
  registry = path.join(directory, 'state', 'files.json');
  files = new ProjectFiles(registry, path.join(directory, 'backups'));
  validate = vi.spyOn(validator, 'validateProjectDocumentInWorker').mockImplementation((bytes, sha256) => {
    buffers.push(bytes); return actualValidate(bytes, sha256, spawn);
  });
});
afterEach(async () => {
  vi.restoreAllMocks();
  await Promise.all(workers.splice(0).map(worker => worker.terminate()));
  await Promise.all(tracked.handles.filter(handle => handle.fd !== -1).map(handle => handle.close()));
  buffers.length = 0;
  await rm(directory, { recursive: true, force: true });
});

/** Real indexed files at the production boundary; no reduced-threshold mock. */
async function fixture(options: { source?: string; length?: number; trailing?: string; checksum?: string; mediaCorrupt?: boolean } = {}) {
  const document = Buffer.alloc(options.length ?? validator.WORKER_DOCUMENT_BYTES, 32);
  document.write(options.source ?? source);
  if (options.trailing) document.write(options.trailing, document.length - Buffer.byteLength(options.trailing));
  const record: ProjectRecord = { offset: 5, length: document.length, sha256: options.checksum ?? digest(document) };
  const mediaBytes = Buffer.from([7, 8, 9]);
  const media = [{ id: 'clip', revision: 'unchanged', type: 'video/webm', offset: 5 + document.length,
    length: mediaBytes.length, sha256: digest(mediaBytes) }];
  const index = Buffer.from(JSON.stringify({ version: 4, document: record, media }));
  const footer = projectFooter(media[0]!.offset + mediaBytes.length, index.length, digest(index));
  if (options.mediaCorrupt) mediaBytes[0] = 0;
  const hash = createHash('sha256').update('PMV4\n').update(document).update(mediaBytes).update(index).update(footer).digest('hex');
  const file = await open(tracked.path, 'w');
  try { await file.writeFile(Buffer.from('PMV4\n')); await file.writeFile(document); await file.writeFile(mediaBytes);
    await file.writeFile(index); await file.writeFile(footer); }
  finally { await file.close(); }
  return { record, media, hash };
}

it('keeps the full default result, uses a real owned worker only when opted in, and preserves reads/overwrite grants', async () => {
  const { record, media, hash } = await fixture();
  const full = await files.open(tracked.path);
  expect(full.document).toEqual(value);
  expect(Object.keys(full)).toEqual(['path', 'projectId', 'token', 'size', 'document', 'media', 'documentRange']);
  expect(full.documentRange).toEqual(record);
  expect(validate).not.toHaveBeenCalled();
  const ranged = await files.open(tracked.path, { allowDocumentRange: true });
  expect(ranged).not.toHaveProperty('document');
  expect(ranged.documentRange).toEqual(record);
  expect(ranged.media).toEqual(media);
  expect(ranged.projectId).not.toBe(full.projectId);
  expect(ranged.projectId).not.toBe('embedded');
  expect(validate).toHaveBeenCalledTimes(1);
  expect(buffers[0]!.byteLength).toBe(0);
  expect(workers[0]!.threadId).toBe(-1);
  expect(JSON.parse(await readFile(registry, 'utf8'))[ranged.projectId]).toEqual({ path: tracked.path, hash });
  const readHash = createHash('sha256');
  for (let offset = 0; offset < record.length; offset += FILE_CHUNK_BYTES) {
    readHash.update(await files.read(ranged.token, record.offset + offset, Math.min(FILE_CHUNK_BYTES, record.length - offset)));
  }
  expect(readHash.digest('hex')).toBe(record.sha256);
  expect([...await files.read(ranged.token, media[0]!.offset, 3)]).toEqual([7, 8, 9]);
  await expect(files.read(ranged.token, 5, FILE_CHUNK_BYTES + 1)).rejects.toThrow('range');
  await expect(files.read('invented', 5, 1)).rejects.toThrow('range');
  await files.close(full.token); await files.close(ranged.token);
  expect(tracked.handles.every(handle => handle.fd === -1)).toBe(true);
  const edited = Buffer.from(JSON.stringify({ ...value, proj: { ...value.proj, name: 'edited' } }));
  await files.save(ranged.projectId, edited);
  expect(await readFile(tracked.path)).toEqual(edited);
  await expect(files.save(full.projectId, Buffer.from(source))).rejects.toThrow('outside Powermove');
}, 15_000);

it('opens a large range stamped for a newer reader without a save association, and says why a newer one cannot open', async () => {
  const newer = { ...value, proj: { ...value.proj, minReaderVersion: 2 } };
  await fixture({ source: JSON.stringify(newer) });
  const ranged = await files.open(tracked.path, { allowDocumentRange: true });
  expect(ranged).not.toHaveProperty('document');
  expect(validate).toHaveBeenCalledTimes(1);
  // Read-only: no association, so no save can target this file without Save As.
  expect(JSON.parse(await readFile(registry, 'utf8').catch(() => '{}'))[ranged.projectId]).toBeUndefined();
  await expect(files.save(ranged.projectId, Buffer.from(source))).rejects.toThrow('Save As');
  await files.close(ranged.token);
  await fixture({ source: JSON.stringify({ minReaderVersion: 5, proj: { timelines: [] } }) });
  await expect(files.open(tracked.path, { allowDocumentRange: true })).rejects.toThrow('This project needs a newer version of Powermove.');
  expect(validate).toHaveBeenCalledTimes(2);
});

it('keeps the measured boundary, small PMV4, PMV3, and legacy opens on the full native path', async () => {
  await fixture({ length: validator.WORKER_DOCUMENT_BYTES - 1 });
  const below = await files.open(tracked.path, { allowDocumentRange: true });
  expect(below.document).toEqual(value); await files.close(below.token);
  for (const bytes of [encodeProjectContainer(value, []), Buffer.from(source)]) {
    await writeFile(tracked.path, bytes);
    const older = await files.open(tracked.path, { allowDocumentRange: true });
    expect(older.document).toEqual(value); expect(older).not.toHaveProperty('documentRange');
    await files.close(older.token);
  }
  expect(validate).not.toHaveBeenCalled();
  expect(tracked.handles.every(handle => handle.fd === -1)).toBe(true);
});

it.each(['startup', 'unexpected exit'])('falls back to complete native validation after worker %s without losing the document or bytes', async failure => {
  const { record } = await fixture();
  validate.mockImplementation((bytes: Buffer, sha256: string) => {
    buffers.push(bytes);
    return actualValidate(bytes, sha256, (_script, options) => {
      if (failure === 'startup') throw new Error('Worker unavailable');
      return spawn('process.exit(7)', options);
    });
  });
  const opened = await files.open(tracked.path, { allowDocumentRange: true });
  expect(opened.document).toEqual(value);
  expect(opened.documentRange).toEqual(record);
  expect(buffers[0]!.byteLength).toBe(failure === 'startup' ? record.length : 0);
  expect(workers.every(worker => worker.threadId === -1)).toBe(true);
  const bytes = await files.read(opened.token, record.offset, source.length);
  expect(Buffer.from(bytes).toString()).toBe(source);
  await files.close(opened.token);
  validate.mockImplementation((bytes: Buffer, sha256: string) => actualValidate(bytes, sha256, spawn));
  const retried = await files.open(tracked.path, { allowDocumentRange: true });
  expect(retried).not.toHaveProperty('document');
  expect(retried.documentRange).toEqual(record); await files.close(retried.token);
  expect(tracked.handles.every(handle => handle.fd === -1)).toBe(true);
});

it.each(['checksum', 'syntax', 'shape', 'media'])('rejects large %s errors before persisting associations or granting readers', async invalid => {
  await fixture({ ...(invalid === 'checksum' ? { checksum: '0'.repeat(64), trailing: 'x' } : {}),
    ...(invalid === 'syntax' ? { trailing: 'x' } : {}),
    ...(invalid === 'shape' ? { source: '{"proj":{"w":100,"h":100,"layers":false}}' } : {}),
    ...(invalid === 'media' ? { mediaCorrupt: true } : {}) });
  const error = await files.open(tracked.path, { allowDocumentRange: true }).catch(error => error);
  if (invalid === 'syntax') expect(error).toBeInstanceOf(SyntaxError);
  else expect(error.message).toMatch(invalid === 'shape' ? /not a Powermove project/ : new RegExp(`${invalid === 'media' ? 'media' : 'document'} checksum`));
  expect(workers.every(worker => worker.threadId === -1)).toBe(true);
  expect(tracked.handles.every(handle => handle.fd === -1)).toBe(true);
  await expect(readFile(registry)).rejects.toMatchObject({ code: 'ENOENT' });
});

it('preserves native checksum priority when a worker cannot start and the malformed document is retried', async () => {
  await fixture({ checksum: '0'.repeat(64), trailing: 'x' });
  validate.mockImplementation((bytes: Buffer, sha256: string) => actualValidate(bytes, sha256, () => { throw new Error('Unavailable'); }));
  await expect(files.open(tracked.path, { allowDocumentRange: true })).rejects.toMatchObject({
    name: 'Error', message: 'The project document checksum does not match.',
  });
  expect(tracked.handles.every(handle => handle.fd === -1)).toBe(true);
  await expect(readFile(registry)).rejects.toMatchObject({ code: 'ENOENT' });
});

it('keeps document errors first and cancels parallel hashing after its pending read settles', async () => {
  await fixture({ trailing: 'x' });
  let release!: () => void, started!: () => void, hashSettled = false;
  const pending = new Promise<void>(resolve => { release = resolve; });
  const reading = new Promise<void>(resolve => { started = resolve; });
  let reads = 0, signal: AbortSignal | undefined;
  const hashing = vi.spyOn(projectHash, 'hashProjectFile').mockImplementation(async (_handle, _size, media, abort) => {
    signal = abort;
    try {
      return await actualHash({ read: async (buffer: Buffer, offset: number, length: number) => {
        reads++; started(); await pending; buffer.fill(32, offset, offset + length);
        return { bytesRead: length, buffer };
      } } as FileHandle, 1024 * 1024 * 1024, [], abort);
    } finally { hashSettled = true; }
  });
  let finished = false;
  const opened = files.open(tracked.path, { allowDocumentRange: true }).then(() => { finished = true; }, error => { finished = true; return error; });
  try {
    await reading;
    await vi.waitFor(() => expect(signal?.aborted).toBe(true));
    expect(finished).toBe(false); expect(hashSettled).toBe(false);
    expect(tracked.handles).toHaveLength(1); expect(tracked.handles[0]!.fd).not.toBe(-1);
  } finally { release(); }
  const error = await opened;
  expect(error).toBeInstanceOf(SyntaxError);
  expect(reads).toBe(1); expect(hashing).toHaveBeenCalledTimes(1); expect(hashSettled).toBe(true);
  expect(tracked.handles[0]!.fd).toBe(-1);
  expect(workers.every(worker => worker.threadId === -1)).toBe(true);
  await expect(readFile(registry)).rejects.toMatchObject({ code: 'ENOENT' });
});

it('captures early parallel hash rejection and lets complete document validation take priority', async () => {
  await fixture({ source: '{"proj":{"w":1,"h":1,"layers":[]},"unused":[1,]}' });
  vi.spyOn(projectHash, 'hashProjectFile').mockRejectedValue(new Error('Parallel media read failed first'));
  await expect(files.open(tracked.path, { allowDocumentRange: true })).rejects.toBeInstanceOf(SyntaxError);
  expect(tracked.handles.every(handle => handle.fd === -1)).toBe(true);
  expect(workers.every(worker => worker.threadId === -1)).toBe(true);
});

it('checks the file version after worker validation before creating an overwrite association', async () => {
  await fixture();
  let release!: () => void, started!: () => void, hashed!: () => void;
  const pending = new Promise<void>(resolve => { release = resolve; }), validation = new Promise<void>(resolve => { started = resolve; });
  const hashComplete = new Promise<void>(resolve => { hashed = resolve; });
  vi.spyOn(projectHash, 'hashProjectFile').mockImplementation(async (...args) => {
    const hash = await actualHash(...args); hashed(); return hash;
  });
  validate.mockImplementation(async (bytes: Buffer, sha256: string) => { started(); await pending; return actualValidate(bytes, sha256, spawn); });
  const opened = files.open(tracked.path, { allowDocumentRange: true });
  const result = opened.catch(error => error);
  try {
    await validation; await hashComplete;
    const foreign = await open(tracked.path, 'r+');
    try { await foreign.write(Buffer.from('\n'), 0, 1, 5 + source.length); }
    finally { await foreign.close(); }
  }
  finally { release(); }
  const error = await result;
  expect(error.message).toBe('The project changed while opening. Try opening it again.');
  expect(workers.every(worker => worker.threadId === -1)).toBe(true);
  expect(tracked.handles.every(handle => handle.fd === -1)).toBe(true);
  await expect(readFile(registry)).rejects.toMatchObject({ code: 'ENOENT' });
  const foreign = await open(tracked.path, 'r');
  try {
    const buffer = Buffer.alloc(1); await foreign.read(buffer, 0, 1, 5 + source.length);
    expect(buffer.toString()).toBe('\n');
  } finally { await foreign.close(); }
});

it('finishes worker validation and closes the granted reader when the window closes during opening', async () => {
  await fixture();
  let release!: () => void, started!: () => void, destroyed = false;
  const pending = new Promise<void>(resolve => { release = resolve; }), validation = new Promise<void>(resolve => { started = resolve; });
  validate.mockImplementation(async (bytes: Buffer, sha256: string) => { started(); await pending; return actualValidate(bytes, sha256, spawn); });
  const owner = Object.assign(new EventEmitter(), { isDestroyed: () => destroyed });
  const opened = openProjectForWindow({ projects: files }, owner as any, tracked.path);
  try { await validation; destroyed = true; owner.emit('destroyed'); }
  finally { release(); }
  expect(await opened).toEqual({ ok: false, cancelled: false, error: 'Project window is unavailable' });
  expect(workers.every(worker => worker.threadId === -1)).toBe(true);
  expect(tracked.handles.every(handle => handle.fd === -1)).toBe(true);
  expect(owner.listenerCount('destroyed')).toBe(0);
});
