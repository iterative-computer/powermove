import { createHash } from 'node:crypto';
import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, it } from 'vitest';
import { canonicalStringify } from '../../shared/canonical-json';
import { DocumentRecovery } from './document-recovery';
const hash = (s: string) => createHash('sha256').update(s).digest('hex');
/** The canonical hash of the tree `text` parses to (pm-model: sorted keys). */
const canonicalHash = (text: string) => hash(canonicalStringify(JSON.parse(text))!);
async function fixture() {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'pm-recovery-'));
  let generation = 1, revision = 0, value = 0, next = 0, prepared: any, persisted: any, lose = false;
  let ready = Promise.resolve(), resume = () => {}, ids = new Map<number, string>(), context: string[] = [];
  const ops: string[] = [], loads: string[] = [];
  const kill = (block = false) => { generation++; revision = 0; value = 0; ids = new Map(); context = []; next = 0; if (block) ready = new Promise(resolve => { resume = resolve; }); };
  const recovery = new DocumentRecovery({ cache: path.join(directory, 'cache'), ready: async () => { await ready; return generation; },
    call: async (_, json, bytes) => {
      const { op, args } = JSON.parse(json); ops.push(op); let answer: any = null;
      if (op === 'load') {
        // As the utility (pm-node kernel.rs): a text source is checked against
        // its bytes' hash; a canonical one against the canonical hash of the
        // tree it parses to. Both are answered with that canonical hash.
        const text = await readFile(args.path, 'utf8');
        if ((args.source === 'text' ? hash(text) : canonicalHash(text)) !== args.hash) return JSON.stringify({ ok: false, error: 'hash' });
        loads.push(args.source ?? 'canonical');
        prepared = { pending: false, revision: args.revision, hash: canonicalHash(text), integrity: 'checked', value: JSON.parse(text).value }; answer = prepared;
      }
      else if (op === 'resync') { value = JSON.parse(args.json).value; revision = args.revision; }
      else if (op === 'poll') answer = prepared;
      else if (op === 'activate') { if (args.hash !== prepared.hash) return JSON.stringify({ ok: false, error: 'activation hash' }); value = prepared.value; revision = args.revision; }
      else if (op === 'apply') { expect(args.base).toBe(revision); value += JSON.parse(Buffer.from(bytes!).toString()).increment; revision = args.revision; if (lose) { lose = false; kill(); return '{"ok":false,"code":"unavailable"}'; } answer = { code: 0 }; }
      else if (op === 'intern') { const key = JSON.stringify(args), old = [...ids].find(([, s]) => key === s); answer = old?.[0] ?? next++; ids.set(answer, key); }
      else if (op === 'release') ids.delete(args.id);
      else if (op === 'ev') { expect(ids.has(args.id)).toBe(true); answer = value; }
      else if (op === 'toJson') answer = JSON.stringify({ value });
      else if (op === 'revision') answer = revision;
      else if (op === 'persist') { const text = JSON.stringify({ value }); await writeFile(args.path, text); persisted = { pending: false, revision, hash: hash(text), integrity: 'checked' }; answer = { pending: true }; }
      else if (op === 'pollPersist') answer = persisted;
      else context.push(json + (bytes ? ':' + Buffer.from(bytes).toString('hex') : ''));
      return JSON.stringify({ ok: true, value: answer });
    }
  });
  const call = async (op: string, args?: any, bytes?: Uint8Array) => { const r = JSON.parse(await recovery.call(7, JSON.stringify({ op, args }), bytes)); expect(r.ok).toBe(true); return r.value; };
  return { call, kill, resume: () => resume(), ops, loads, context: () => context, lose: () => { lose = true; },
    async openText() {
      const file = path.join(directory, 'staging'), text = '{ "value": 1 }';
      await writeFile(file, text);
      const receipt = await call('load', { path: file, document: 'test', revision: 1, hash: hash(text), source: 'text' });
      await rm(file);
      await call('activate', { revision: 1, preparedRevision: 1, hash: receipt.hash });
    },
    async open() { const file = path.join(directory, 'staging'), text = '{"value":1}'; await writeFile(file, text); await call('load', { path: file, document: 'test', revision: 1, hash: hash(text) }); await rm(file); await call('activate', { revision: 1, preparedRevision: 1, hash: hash(text) }); },
    async count() { const root = path.join(directory, 'cache'); const dirs = await readdir(root); return dirs.length ? (await readdir(path.join(root, dirs[0]!))).length : 0; },
    async stop() { await recovery.dispose(); expect(await readdir(path.join(directory, 'cache'))).toEqual([]); await rm(directory, { recursive: true, force: true }); }
  };
}

it('queues reads and writes across utility loss with no renderer work and preserves live handles and context', async () => {
  const f = await fixture();
  try {
    await f.open(); const discarded = await f.call('intern', { key: 'discard' }), held = await f.call('intern', { key: 'held' });
    await f.call('release', { id: discarded }); await f.call('define', { id: 'shape', label: 'Shape' }); await f.call('addFont', { blob: 'face', index: 0 }, new Uint8Array([1, 2]));
    await f.call('apply', { base: 1, revision: 2 }, Buffer.from('{"increment":2}'));
    f.kill(true); let answered = false;
    const read = f.call('ev', { id: held, t: 0 }).then(v => { answered = true; return v; });
    const first = f.call('apply', { base: 2, revision: 3 }, Buffer.from('{"increment":3}'));
    const second = f.call('apply', { base: 3, revision: 4 }, Buffer.from('{"increment":4}'));
    await Promise.resolve(); expect(answered).toBe(false); f.resume(); expect(await read).toBe(3); await Promise.all([first, second]);
    expect(await f.call('toJson')).toBe('{"value":10}'); expect(await f.call('ev', { id: held, t: 0 })).toBe(10);
    expect(f.context().some(s => s.includes('Shape'))).toBe(true); expect(f.context().some(s => s.endsWith(':0102'))).toBe(true);
    expect(await f.call('intern', { key: 'new' })).not.toBe(held);
  } finally { await f.stop(); }
});
it('applies an uncertain write exactly once after a kill before acknowledgment', async () => {
  const f = await fixture(); try { await f.open(); f.lose(); await f.call('apply', { base: 1, revision: 2 }, Buffer.from('{"increment":7}')); expect(await f.call('toJson')).toBe('{"value":8}'); } finally { await f.stop(); }
});
it('compacts a bounded suffix and deduplicates context before recovery', async () => {
  const f = await fixture(); try {
    await f.open();
    for (let i = 1; i <= 132; i++) await f.call('apply', { base: i, revision: i + 1 }, Buffer.from('{"increment":1}'));
    for (let i = 0; i < 12; i++) { await f.call('define', { id: 'shape', label: String(i) }); await f.call('addFontAnswers', { json: JSON.stringify({ faces: [{ id: 1 }], answers: [{ query: { text: 'a' }, face: 1, advance: i }] }) }); }
    expect(f.ops.filter(op => op === 'persist')).toHaveLength(1); expect(await f.count()).toBeLessThan(10);
    f.kill(); expect(await f.call('toJson')).toBe('{"value":133}'); expect(f.context()).toHaveLength(3);
    await f.call('close'); expect(await f.count()).toBe(0);
  } finally { await f.stop(); }
});
it('restores a document received as JSON text with its byte hash, then activates it by its canonical hash', async () => {
  const f = await fixture(); try {
    await f.openText();
    await f.call('apply', { base: 1, revision: 2 }, Buffer.from('{"increment":4}'));
    f.kill();
    expect(await f.call('toJson')).toBe('{"value":5}');
    expect(f.loads).toEqual(['text', 'text']);
  } finally { await f.stop(); }
});
it('restores a resync base, whose JSON is not in canonical byte order, by its bytes and then its canonical hash', async () => {
  const f = await fixture(); try {
    await f.open();
    // The barrier's resync body: the canonical project form, but JSON.stringify's member order.
    const json = '{"value":6,"name":"resynced"}';
    expect(canonicalStringify(JSON.parse(json))).not.toBe(json);
    await f.call('resync', { json, revision: 2, document: 'test' });
    await f.call('apply', { base: 2, revision: 3 }, Buffer.from('{"increment":3}'));
    f.kill();
    expect(await f.call('toJson')).toBe('{"value":9}');
    expect(await f.call('revision')).toBe(3);
    expect(f.loads).toEqual(['canonical', 'text']);
    // Recovered again: now checked by the canonical hash the first restore answered.
    f.kill();
    expect(await f.call('toJson')).toBe('{"value":9}');
    expect(f.loads).toEqual(['canonical', 'text', 'text']);
  } finally { await f.stop(); }
});
