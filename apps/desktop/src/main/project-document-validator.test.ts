import { EventEmitter } from 'node:events';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { Worker } from 'node:worker_threads';
import { build } from 'esbuild';
import { afterEach, expect, it, vi } from 'vitest';
import { assertProjectDocument, ProjectDocumentWorkerError, validateProjectDocumentInWorker } from './project-document-validator';

const workers: Worker[] = [], directories: string[] = [];
afterEach(async () => {
  await Promise.all(workers.splice(0).map(worker => worker.terminate()));
  await Promise.all(directories.splice(0).map(directory => rm(directory, { recursive: true, force: true })));
});
const digest = (bytes: Uint8Array) => createHash('sha256').update(bytes).digest('hex');
function owned(input: string | Uint8Array) {
  const original = typeof input === 'string' ? Buffer.from(input) : Buffer.from(input);
  const buffer = Buffer.allocUnsafeSlow(original.length); original.copy(buffer);
  return buffer;
}
const spawn = vi.fn((source: string, options: ConstructorParameters<typeof Worker>[1]) => {
  const worker = new Worker(source, options); workers.push(worker); return worker;
});

it.each([
  '{"proj":{"w":100,"h":100,"layers":[]},"history":{"entries":[{"label":"Move"}]}}',
  '{"w":0,"h":-10,"layers":[],"value":1e400}',
  '{"proj":{"layers":false},"proj":{"w":320,"h":180,"layers":[]}}',
  '{"\\u0070roj":{"\\u0077":100,"h":100,"layers":[],"__proto__":{"safe":true},"constructor":1}}',
  '{"proj":null,"w":100,"h":100,"layers":[]}',
])('uses native complete-document semantics for %s and releases the real worker', async source => {
  assertProjectDocument(JSON.parse(source));
  const buffer = owned(source), sha256 = digest(buffer);
  await validateProjectDocumentInWorker(buffer, sha256, spawn);
  expect(buffer.byteLength).toBe(0);
  expect(workers.at(-1)!.threadId).toBe(-1);
});

it.each([
  '', 'null', 'true', '{}', '{"proj":{"w":"100","h":100,"layers":[]}}',
  '{"proj":{"w":1e400,"h":100,"layers":[]}}',
  '{"proj":{"w":100,"h":100,"layers":[]},"proj":{"layers":[]}}',
  '{"proj":{"w":100,"h":100,"layers":[]}} trailing',
  '{"proj":{"w":100,"h":100,"layers":[]},"unused":[1,]}',
  '{"proj":{"w":100,"h":100,"layers":[]},"text":"\\x"}',
])('preserves the native error name/message for %s after worker exit', async source => {
  let expected: Error | undefined;
  try { assertProjectDocument(JSON.parse(source)); } catch (error) { expected = error as Error; }
  expect(expected).toBeDefined();
  const buffer = owned(source);
  await expect(validateProjectDocumentInWorker(buffer, digest(buffer), spawn)).rejects.toMatchObject({
    name: expected!.name, message: expected!.message,
  });
  expect(workers.at(-1)!.threadId).toBe(-1);
});

it('checks the checksum before malformed JSON and preserves Node UTF-8 decoding', async () => {
  const invalid = owned('{not json');
  await expect(validateProjectDocumentInWorker(invalid, '0'.repeat(64), spawn)).rejects.toMatchObject({
    name: 'Error', message: 'The project document checksum does not match.',
  });
  const invalidUtf8 = Buffer.concat([Buffer.from('{"proj":{"w":1,"h":2,"layers":[]},"text":"'), Buffer.from([255]), Buffer.from('"}')]);
  assertProjectDocument(JSON.parse(invalidUtf8.toString('utf8')));
  const buffer = owned(invalidUtf8);
  await validateProjectDocumentInWorker(buffer, digest(buffer), spawn);
  const bom = owned(Buffer.concat([Buffer.from([239, 187, 191]), Buffer.from('{"w":1,"h":2,"layers":[]}')]));
  let expected: Error | undefined;
  try { JSON.parse(bom.toString('utf8')); } catch (error) { expected = error as Error; }
  await expect(validateProjectDocumentInWorker(bom, digest(bom), spawn)).rejects.toMatchObject({ name: expected!.name, message: expected!.message });
  expect(workers.every(worker => worker.threadId === -1)).toBe(true);
});

it('refuses pooled/offset buffers before loading or constructing a worker', async () => {
  const backing = Buffer.allocUnsafeSlow(100), offset = backing.subarray(1, 50), create = vi.fn();
  await expect(validateProjectDocumentInWorker(offset, digest(offset), create)).rejects.toBeInstanceOf(ProjectDocumentWorkerError);
  expect(create).not.toHaveBeenCalled();
  expect(backing.byteLength).toBe(100);
});

class FakeWorker extends EventEmitter {
  terminate = vi.fn(async () => { this.emit('exit', 1); return 1; });
}
function fake(worker: FakeWorker, event: string, value: unknown) {
  return () => { queueMicrotask(() => worker.emit(event, value)); return worker as unknown as Worker; };
}

it.each(['success', 'invalid'])('awaits termination/exit after a %s acknowledgement and removes all listeners', async mode => {
  const worker = new FakeWorker();
  let stop!: () => void, finished = false;
  const exit = new Promise<void>(resolve => { stop = resolve; });
  worker.terminate.mockImplementation(async () => { await exit; worker.emit('exit', 1); return 1; });
  const response = mode === 'success' ? { ok: true } : { ok: false, error: { name: 'SyntaxError', message: 'Native syntax failure' } };
  const buffer = owned('{}');
  const validation = validateProjectDocumentInWorker(buffer, digest(buffer), fake(worker, 'message', response));
  const result = validation.then(() => { finished = true; }, error => { finished = true; return error; });
  await vi.waitFor(() => expect(worker.terminate).toHaveBeenCalledTimes(1));
  expect(finished).toBe(false);
  stop();
  const error = await result;
  if (mode === 'invalid') expect(error).toBeInstanceOf(SyntaxError);
  else expect(error).toBeUndefined();
  expect(worker.eventNames()).toEqual([]);
});

it.each(['error', 'exit', 'message'])('classifies an unexpected %s as infrastructure failure after cleanup', async event => {
  const worker = new FakeWorker(), buffer = owned('{}');
  const value = event === 'error' ? new Error('Worker unavailable') : event === 'exit' ? 0 : { document: {} };
  await expect(validateProjectDocumentInWorker(buffer, digest(buffer), fake(worker, event, value))).rejects.toBeInstanceOf(ProjectDocumentWorkerError);
  expect(worker.terminate).toHaveBeenCalledTimes(1);
  expect(worker.eventNames()).toEqual([]);
});

it('captures startup failure and actual unexpected worker exit after transferring bytes', async () => {
  const buffer = owned('{}');
  await expect(validateProjectDocumentInWorker(buffer, digest(buffer), () => { throw new Error('No worker'); })).rejects.toBeInstanceOf(ProjectDocumentWorkerError);
  expect(buffer.byteLength).toBe(2);
  const transferred = owned('{}');
  await expect(validateProjectDocumentInWorker(transferred, digest(transferred), (_source, options) => {
    const worker = new Worker('process.exit(7)', options); workers.push(worker); return worker;
  })).rejects.toBeInstanceOf(ProjectDocumentWorkerError);
  expect(transferred.byteLength).toBe(0);
  expect(workers.at(-1)!.threadId).toBe(-1);
});

it('waits for exit after teardown failure and preserves a prior expected syntax error', async () => {
  for (const valid of [true, false]) {
    const worker = new FakeWorker(), buffer = owned('{}');
    worker.terminate.mockImplementation(async () => { queueMicrotask(() => worker.emit('exit', 0)); throw new Error('Teardown fault'); });
    const response = valid ? { ok: true } : { ok: false, error: { name: 'SyntaxError', message: 'Expected syntax failure' } };
    await expect(validateProjectDocumentInWorker(buffer, digest(buffer), fake(worker, 'message', response)))
      .rejects.toBeInstanceOf(valid ? ProjectDocumentWorkerError : SyntaxError);
    expect(worker.eventNames()).toEqual([]);
  }
});

it('validates through a minified keepNames bundle with lazy Worker loading and no fallback', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'powermove-validator-bundle-')); directories.push(directory);
  const module = path.join(directory, 'validator.cjs');
  await build({ entryPoints: [path.join(import.meta.dirname, 'project-document-validator.ts')], outfile: module,
    bundle: true, platform: 'node', format: 'cjs', target: 'es2022', minify: true, keepNames: true });
  const result = JSON.parse(execFileSync(process.execPath, ['-e', `
    const workerLoaded = () => process.moduleLoadList.includes('NativeModule worker_threads');
    const { validateProjectDocumentInWorker, ProjectDocumentWorkerError } = require(${JSON.stringify(module)});
    if (workerLoaded()) throw new Error('Worker loaded during module startup');
    (async () => {
      const value = Buffer.from('{"proj":{"w":1,"h":2,"layers":[]}}');
      const bytes = Buffer.allocUnsafeSlow(value.length); value.copy(bytes);
      const sha = require('node:crypto').createHash('sha256').update(bytes).digest('hex');
      await validateProjectDocumentInWorker(bytes, sha);
      if (!workerLoaded() || bytes.byteLength !== 0) throw new Error('Minified worker did not run');
      const create = text => {
        const value = Buffer.from(text), bytes = Buffer.allocUnsafeSlow(value.length); value.copy(bytes);
        return { bytes, sha: require('node:crypto').createHash('sha256').update(bytes).digest('hex') };
      };
      const errors = [];
      for (const [text, badHash, expectedName, expectedMessage] of [
        ['{not json', true, 'Error', 'The project document checksum does not match.'],
        ['{"proj":{"w":1,"h":2,"layers":[]}} trailing', false, 'SyntaxError', null],
        ['{"proj":{"w":1,"h":2,"layers":false}}', false, 'Error', 'This is not a Powermove project.'],
      ]) {
        const sample = create(text);
        let expected = expectedMessage;
        if (!expected) { try { JSON.parse(text); } catch (error) { expected = error.message; } }
        try { await validateProjectDocumentInWorker(sample.bytes, badHash ? '0'.repeat(64) : sample.sha); throw new Error('Invalid document accepted'); }
        catch (error) {
          if (error.name !== expectedName || error.message !== expected || sample.bytes.byteLength !== 0) throw error;
          errors.push(error.name);
        }
      }
      const retry = create('{"w":1,"h":2,"layers":[]}');
      try { await validateProjectDocumentInWorker(retry.bytes, retry.sha, () => { throw new Error('Unavailable'); }); throw new Error('Startup failure lost'); }
      catch (error) { if (!(error instanceof ProjectDocumentWorkerError) || !retry.bytes.byteLength) throw error; }
      await validateProjectDocumentInWorker(retry.bytes, retry.sha);
      if (retry.bytes.byteLength) throw new Error('Infrastructure retry did not run');
      console.log(JSON.stringify({ valid: true, loaded: workerLoaded(), errors, retried: true }));
    })().catch(error => { console.error(error); process.exitCode = 1; });
  `], { encoding: 'utf8', timeout: 10_000 }));
  expect(result).toEqual({ valid: true, loaded: true, errors: ['Error', 'SyntaxError', 'Error'], retried: true });
}, 15_000);
