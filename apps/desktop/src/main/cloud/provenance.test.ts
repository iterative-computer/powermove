import * as fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { createProvenanceStore, PROVENANCE_FILE } from './provenance';

const roots: string[] = [];
async function tempDir(): Promise<string> {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'powermove-provenance-'));
  roots.push(dir);
  return dir;
}
afterEach(async () => {
  await Promise.all(roots.splice(0).map((dir) => fs.rm(dir, { recursive: true, force: true })));
});

describe('provenance', () => {
  it('creates a local env key once and persists it', async () => {
    const dir = await tempDir();
    let n = 0;
    const store = createProvenanceStore(dir, { uuid: () => `uuid-${++n}` });
    const [first, second] = await Promise.all([store.ensureEnvKey('glass-blur'), store.ensureEnvKey('glass-blur')]);
    expect(first).toBe('local:uuid-1');
    expect(second).toBe('local:uuid-1');
    const reopened = createProvenanceStore(dir, { uuid: () => 'never' });
    expect(await reopened.ensureEnvKey('glass-blur')).toBe('local:uuid-1');
    expect(await reopened.get('glass-blur')).toEqual({ localId: 'glass-blur', envKey: 'local:uuid-1' });
    expect(JSON.parse(await fs.readFile(path.join(dir, PROVENANCE_FILE), 'utf8'))).toEqual({
      'glass-blur': { localId: 'glass-blur', envKey: 'local:uuid-1' }
    });
    expect((await fs.readdir(dir)).filter((name) => name.endsWith('.tmp'))).toEqual([]);
  });

  it('keeps other fields and uses the origin repo for a store install', async () => {
    const dir = await tempDir();
    const origin = { repoId: 'repo_1', releaseId: 'rel_1', coordinate: 'mara/glass', version: '1.0.0', treeSha: 't', commitSha: 'c', ownerPublisherId: 'p' };
    await fs.writeFile(path.join(dir, PROVENANCE_FILE), JSON.stringify({
      glass: { localId: 'glass', origin },
      other: { localId: 'other', envKey: 'local:x', future: { kept: true } }
    }));
    const store = createProvenanceStore(dir);
    expect(await store.get('glass')).toBeNull();
    expect(await store.ensureEnvKey('glass')).toBe('repo_1');
    const saved = JSON.parse(await fs.readFile(path.join(dir, PROVENANCE_FILE), 'utf8'));
    expect(saved.glass).toEqual({ localId: 'glass', origin, envKey: 'repo_1' });
    expect(saved.other.future).toEqual({ kept: true });
  });

  it('removes an entry and refuses a damaged file instead of overwriting it', async () => {
    const dir = await tempDir();
    const store = createProvenanceStore(dir);
    await store.ensureEnvKey('one');
    await store.ensureEnvKey('two');
    await store.remove('one');
    expect(Object.keys(await store.read())).toEqual(['two']);
    await fs.writeFile(path.join(dir, PROVENANCE_FILE), '{not json');
    await expect(store.ensureEnvKey('three')).rejects.toThrow('Powermove will treat store extensions as untrusted until it is repaired.');
    expect(await fs.readFile(path.join(dir, PROVENANCE_FILE), 'utf8')).toBe('{not json');
    await expect(store.get('../x')).rejects.toThrow(/Invalid extension id/);
  });

  it('returns complete caller results and continues queued writes in order after a failure', async () => {
    const dir = await tempDir(), store = createProvenanceStore(dir);
    const initial = { localId: 'serial', envKey: 'local:serial', future: { steps: ['first'], kept: { nested: true } } };
    const order: string[] = [], failure = new Error('Synthetic mutation failed');
    const first = store.update('serial', () => { order.push('first'); return initial; });
    const failed = store.update('serial', () => { order.push('failed'); throw failure; });
    const rejected = expect(failed).rejects.toBe(failure);
    const last = store.update('serial', current => {
      order.push('last');
      const previous = current as typeof initial;
      return { ...previous, future: { ...previous.future, steps: [...previous.future.steps, 'last'] } };
    });
    expect(await first).toEqual(initial);
    await rejected;
    expect(await last).toEqual({ ...initial, future: { ...initial.future, steps: ['first', 'last'] } });
    expect(order).toEqual(['first', 'failed', 'last']);
    const result = await store.read();
    expect(result.serial).toEqual({ ...initial, future: { ...initial.future, steps: ['first', 'last'] } });
    expect(initial.future.steps).toEqual(['first']);
  });
});
