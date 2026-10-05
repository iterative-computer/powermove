import { createHash } from 'node:crypto';
import { createServer, type Server } from 'node:http';
import { mkdtemp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import type { AddressInfo } from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import type { CatalogModel } from './catalog';
import { ModelStore, type FetchLike } from './models';

const sha = (data: Buffer) => createHash('sha256').update(data).digest('hex');
const bytes = (size: number, seed: number) => Buffer.from(Array.from({ length: size }, (_, index) => (index * 31 + seed) % 251));

interface Served { body: Buffer; requests: Array<string | undefined>; ignoreRange?: boolean; stallAfter?: number }

let server: Server;
let origin = '';
let files: Record<string, Served> = {};
let root = '';

beforeEach(async () => {
  root = await mkdtemp(path.join(os.tmpdir(), 'pm-models-'));
  files = {};
  server = createServer((request, response) => {
    const served = files[request.url ?? ''];
    if (!served) { response.writeHead(404).end(); return; }
    served.requests.push(request.headers.range);
    const range = /bytes=(\d+)-/.exec(request.headers.range ?? '');
    const from = range && !served.ignoreRange ? Number(range[1]) : 0;
    if (from >= served.body.length && range) { response.writeHead(416).end(); return; }
    response.writeHead(range && !served.ignoreRange ? 206 : 200, { 'Content-Length': String(served.body.length - from) });
    const body = served.body.subarray(from);
    if (served.stallAfter !== undefined) {
      response.write(body.subarray(0, served.stallAfter));
      return; // never ends
    }
    response.end(body);
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  origin = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
});

afterEach(async () => {
  server.closeAllConnections();
  await new Promise<void>((resolve) => server.close(() => resolve()));
  await rm(root, { recursive: true, force: true });
});

function model(id: string, entries: Record<string, Buffer>): CatalogModel {
  for (const [name, body] of Object.entries(entries)) files[`/${id}/${name}`] = { body, requests: [] };
  return {
    id, name: id, description: '', family: 'parakeet', languages: 'multi', languageCodes: ['en', 'de'], detectsLanguage: true,
    timing: 'word', speed: 0.8, accuracy: 0.8, recommended: false, featured: false, wordTimestamps: true, license: '',
    files: Object.entries(entries).map(([name, body]) => ({ name, url: `${origin}/${id}/${name}`, size: body.length, sha256: sha(body) }))
  };
}

const nodeFetch: FetchLike = (url, init) => fetch(url, init);

function store(catalog: CatalogModel[], extra: Partial<ConstructorParameters<typeof ModelStore>[0]> = {}) {
  const changes: Array<ReturnType<ModelStore['status']>> = [];
  const instance = new ModelStore({ root, catalog, fetch: nodeFetch, onChange: (status) => changes.push(status), progressIntervalMs: 0, ...extra });
  return { instance, changes };
}

describe('ModelStore', () => {
  it('downloads, verifies and finalizes a model, then makes it active', async () => {
    const encoder = bytes(200_000, 1), tokens = bytes(500, 2);
    const catalog = [model('alpha', { 'encoder.int8.onnx': encoder, 'tokens.txt': tokens })];
    const { instance, changes } = store(catalog);
    await instance.load();
    expect(instance.status()).toMatchObject({ activeModelId: null, available: true, language: 'auto', storageBytes: 0 });
    await instance.download('alpha');
    const status = instance.status();
    expect(status.activeModelId).toBe('alpha');
    expect(status.models[0]).toMatchObject({ id: 'alpha', state: 'ready', size: 200_500 });
    expect(status.storageBytes).toBe(200_500);
    expect(await readFile(path.join(root, 'alpha', 'encoder.int8.onnx'))).toEqual(encoder);
    expect(JSON.parse(await readFile(path.join(root, 'alpha', 'model.json'), 'utf8')).files).toHaveLength(2);
    expect(await readdir(path.join(root, '.partial'))).toEqual([]);
    expect(changes.some((change) => change.models[0]!.state === 'downloading')).toBe(true);
    expect(changes.at(-1)!.models[0]!.state).toBe('ready');

    // A fresh store finds it on disk and remembers the choice.
    const again = store(catalog).instance;
    await again.load();
    expect(again.status()).toMatchObject({ activeModelId: 'alpha' });
    expect(again.activeModel()?.dir).toBe(path.join(root, 'alpha'));
  });

  it('resumes a partial file with a Range request', async () => {
    const encoder = bytes(100_000, 3);
    const catalog = [model('beta', { 'encoder.int8.onnx': encoder })];
    await mkdir(path.join(root, '.partial', 'beta'), { recursive: true });
    await writeFile(path.join(root, '.partial', 'beta', 'encoder.int8.onnx.part'), encoder.subarray(0, 40_000));
    const { instance } = store(catalog);
    await instance.load();
    expect(instance.status().models[0]).toMatchObject({ state: 'absent', downloadedBytes: 40_000 });
    await instance.download('beta');
    expect(files['/beta/encoder.int8.onnx']!.requests).toEqual(['bytes=40000-']);
    expect(await readFile(path.join(root, 'beta', 'encoder.int8.onnx'))).toEqual(encoder);
  });

  it('starts a file over when the server ignores the range', async () => {
    const encoder = bytes(50_000, 4);
    const catalog = [model('gamma', { 'encoder.int8.onnx': encoder })];
    files['/gamma/encoder.int8.onnx']!.ignoreRange = true;
    await mkdir(path.join(root, '.partial', 'gamma'), { recursive: true });
    await writeFile(path.join(root, '.partial', 'gamma', 'encoder.int8.onnx.part'), encoder.subarray(0, 10_000));
    const { instance } = store(catalog);
    await instance.download('gamma');
    expect(await readFile(path.join(root, 'gamma', 'encoder.int8.onnx'))).toEqual(encoder);
  });

  it('rejects a corrupted file and reports the error', async () => {
    const good = bytes(10_000, 5);
    const catalog = [model('delta', { 'tokens.txt': good })];
    files['/delta/tokens.txt']!.body = bytes(10_000, 6);
    const { instance } = store(catalog);
    await expect(instance.download('delta')).rejects.toThrow(/checksum/);
    expect(instance.status().models[0]).toMatchObject({ state: 'error' });
    expect(instance.status().activeModelId).toBeNull();
    // Nothing half-done looks ready, and the bad bytes are gone.
    await expect(readdir(path.join(root, 'delta'))).rejects.toThrow();
    expect(await readdir(path.join(root, '.partial', 'delta'))).toEqual([]);
  });

  it('cancels, keeps what arrived, and removes everything on delete', async () => {
    const encoder = bytes(300_000, 7);
    const catalog = [model('eps', { 'encoder.int8.onnx': encoder })];
    files['/eps/encoder.int8.onnx']!.stallAfter = 120_000;
    const { instance } = store(catalog);
    const running = instance.download('eps');
    for (let tries = 0; tries < 100 && (instance.status().models[0]!.downloadedBytes ?? 0) < 120_000; tries++) await new Promise((resolve) => setTimeout(resolve, 10));
    await instance.cancel('eps');
    await running;
    expect(instance.status().models[0]).toMatchObject({ state: 'absent', downloadedBytes: 120_000 });
    await instance.remove('eps');
    expect(instance.status().models[0]).toMatchObject({ state: 'absent' });
    expect(instance.status().models[0]!.downloadedBytes).toBeUndefined();
    await expect(readdir(path.join(root, '.partial', 'eps'))).rejects.toThrow();
  });

  it('gives up on a stalled connection', async () => {
    const encoder = bytes(80_000, 8);
    const catalog = [model('zeta', { 'encoder.int8.onnx': encoder })];
    files['/zeta/encoder.int8.onnx']!.stallAfter = 1000;
    const { instance } = store(catalog, { stallMs: 150 });
    await expect(instance.download('zeta')).rejects.toThrow(/stalled/);
    expect(instance.status().models[0]).toMatchObject({ state: 'error' });
  });

  it('refuses to start without disk space for the model', async () => {
    const catalog = [model('eta', { 'tokens.txt': bytes(1000, 9) })];
    const { instance } = store(catalog, { freeBytes: async () => 1024 });
    await expect(instance.download('eta')).rejects.toThrow(/Not enough disk space/);
    expect(files['/eta/tokens.txt']!.requests).toEqual([]);
  });

  it('switches models and language, and falls back when the active one is deleted', async () => {
    const catalog = [model('one', { 'tokens.txt': bytes(100, 10) }), model('two', { 'tokens.txt': bytes(100, 11) })];
    const { instance } = store(catalog);
    await instance.download('one');
    await instance.download('two');
    expect(instance.status().activeModelId).toBe('one');
    await expect(instance.setActive('missing')).rejects.toThrow();
    await instance.setActive('two');
    await instance.setLanguage('de');
    await expect(instance.setLanguage('not a tag')).rejects.toThrow();
    expect(instance.status()).toMatchObject({ activeModelId: 'two', language: 'de' });
    await instance.remove('two');
    expect(instance.status().activeModelId).toBe('one');
    const settings = JSON.parse(await readFile(path.join(root, 'settings.json'), 'utf8'));
    expect(settings).toEqual({ activeModelId: 'one', language: 'de' });
  });

  it('does not treat a model whose checksums changed as ready', async () => {
    const catalog = [model('theta', { 'tokens.txt': bytes(100, 12) })];
    await store(catalog).instance.download('theta');
    const changed = [{ ...catalog[0]!, files: catalog[0]!.files.map((file) => ({ ...file, sha256: 'f'.repeat(64) })) }];
    const reopened = store(changed).instance;
    await reopened.load();
    expect(reopened.status()).toMatchObject({ activeModelId: null });
    expect(reopened.status().models[0]!.state).toBe('absent');
    // An older build of the model can never load again: its folder is gone.
    expect(await readdir(root)).not.toContain('theta');
  });

  it('keeps a folder whose manifest still names the current file (a damaged copy, not an old build)', async () => {
    const body = bytes(100, 13);
    const catalog = [model('iota', { 'model.gguf': body })];
    await store(catalog).instance.download('iota');
    await writeFile(path.join(root, 'iota', 'model.gguf'), body.subarray(0, 50));
    const reopened = store(catalog).instance;
    await reopened.load();
    expect(reopened.status().models[0]!.state).toBe('absent');
    expect(await readdir(root)).toContain('iota');
  });

  it('reports timing, language detection and the curated flag, and picks a word-timed model on request', async () => {
    const untimed = { ...model('cohere', { 'model.gguf': bytes(100, 14) }), timing: 'none' as const, wordTimestamps: false, detectsLanguage: false, featured: true };
    const english = { ...model('unified', { 'model.gguf': bytes(100, 15) }), languages: 'en' as const, languageCodes: ['en'] };
    const german = model('nemotron', { 'model.gguf': bytes(100, 16) });
    const { instance } = store([untimed, english, german]);
    await instance.download('cohere');
    expect(instance.status().models[0]).toMatchObject({ timing: 'none', wordTimestamps: false, detectsLanguage: false, featured: true });
    expect(instance.modelFor({ wordTimestamps: true })).toBeNull();
    expect(instance.modelFor()?.model.id).toBe('cohere');
    await instance.download('unified');
    await instance.download('nemotron');
    expect(instance.modelFor({ wordTimestamps: true })?.model.id).toBe('unified');
    expect(instance.modelFor({ wordTimestamps: true, language: 'de-AT' })?.model.id).toBe('nemotron');
    await instance.setActive('nemotron');
    expect(instance.modelFor({ wordTimestamps: true, language: 'en' })?.model.id).toBe('nemotron');
  });
});
