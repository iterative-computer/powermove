/*
 * SAVE-1 model check. The real FileStore (storage.ts), StoreEngine and
 * ProjectDurability run against an in-memory file system on which every
 * operation that changes what a restart reads (a file renamed into place, or
 * unlinked) waits for the checker. A renderer posts a script of slot writes
 * and journal patches; the checker explores, depth first by re-execution,
 * every order in which those posts and the pending disk operations can
 * happen, with one injected fault: a disk operation that fails, a slot write
 * the engine refuses, or the storage worker replaced (its operations
 * abandoned before or after they landed).
 *
 * Every disk state any schedule passes through is a crash point. Each must
 * recover (shared/project-journal.ts recoverProject):
 * - a state the document actually had (after some number of its patches);
 * - never fewer patches than an earlier crash point in the same schedule;
 * - every patch the engine had acknowledged as written by then.
 */
import { describe, expect, it } from 'vitest';

import { recoverProject } from '../shared/project-journal';
import type { StoreChannel, StoreFileSystem } from './storage';
import { createStore } from './storage';
import { StoreEngine, type StoreReply, type StoreRequest } from './store-engine';

const DIR = '/store';
const SLOT = `${DIR}/project.P.json`, JOURNAL = `${DIR}/projectJournal.P.json`;
const visible = (file: string) => file === SLOT || file === JOURNAL;

type Gate = { engine: number; kind: 'rename' | 'unlink'; file: string; payload: string | null; apply: () => void; settle: (error?: Error) => void };

class ModelDisk {
  files = new Map<string, string>();
  gates: Gate[] = [];
  /** The visible files after every change, from the start. */
  states: Array<{ slot: string | null; journal: string | null }> = [];
  dead = new Set<number>();
  constructor(initial: Record<string, string>) {
    for (const [name, data] of Object.entries(initial)) this.files.set(name, data);
    this.record();
  }
  record() { this.states.push({ slot: this.files.get(SLOT) ?? null, journal: this.files.get(JOURNAL) ?? null }); }
  /** The file system one engine sees; a replaced engine's calls never return. */
  fs(engine: number): StoreFileSystem {
    const live = <T>(run: () => T | Promise<T>): Promise<T> => this.dead.has(engine) ? new Promise<T>(() => {}) : Promise.resolve().then(run);
    const missing = (file: string) => Object.assign(new Error(`ENOENT: ${file}`), { code: 'ENOENT' });
    const gate = (kind: Gate['kind'], file: string, payload: string | null, apply: () => void) => this.dead.has(engine) ? new Promise<void>(() => {})
      : new Promise<void>((resolve, reject) => this.gates.push({ engine, kind, file, payload, apply, settle: error => error ? reject(error) : resolve() }));
    return {
      mkdir: (() => live(() => undefined)) as any,
      readdir: (() => live(() => [...this.files.keys()].map(file => file.slice(DIR.length + 1)))) as any,
      readFile: ((file: string) => live(() => { const data = this.files.get(String(file)); if (data === undefined) throw missing(String(file)); return data; })) as any,
      writeFile: ((file: string, data: any) => live(() => { this.files.set(String(file), typeof data === 'string' ? data : Buffer.from(data).toString('utf8')); })) as any,
      open: ((file: string) => live(() => {
        const data = this.files.get(String(file));
        if (data === undefined) throw missing(String(file));
        return {
          sync: async () => {}, close: async () => {},
          read: async (buffer: Uint8Array, offset: number, length: number, position: number) => {
            const bytes = Buffer.from(data, 'utf8').subarray(position, position + length);
            buffer.set(bytes, offset);
            return { bytesRead: bytes.length };
          },
        };
      })) as any,
      rename: ((from: string, to: string) => {
        from = String(from); to = String(to);
        const apply = () => { const data = this.files.get(from); if (data === undefined) throw missing(from); this.files.delete(from); this.files.set(to, data); };
        if (!visible(to)) return live(apply);
        return gate('rename', to, this.files.get(from) ?? null, () => { apply(); this.record(); });
      }) as any,
      unlink: ((file: string) => {
        file = String(file);
        const apply = () => { if (!this.files.has(file)) throw missing(file); this.files.delete(file); };
        if (!visible(file)) return live(apply);
        return gate('unlink', file, null, () => { apply(); this.record(); });
      }) as any,
    } as StoreFileSystem;
  }
}

/** The worker thread: a StoreEngine that can be replaced, as persistence-thread.ts does after a fault. */
class ModelChannel implements StoreChannel {
  engine!: StoreEngine;
  generation = 0;
  private listener?: (reply: StoreReply) => void;
  private restartListener?: () => void;
  constructor(private readonly disk: ModelDisk, private readonly observe: (reply: StoreReply) => void) { this.start(); }
  private start() {
    const generation = ++this.generation;
    let suffix = 0;
    this.engine = new StoreEngine({
      fs: this.disk.fs(generation), now: () => 0, randomSuffix: () => `g${generation}s${++suffix}`,
      later: run => { const timer = { cancelled: false }; queueMicrotask(() => { if (!timer.cancelled) run(); }); return timer as any; },
      cancel: timer => { (timer as any).cancelled = true; },
    }, reply => {
      if (generation !== this.generation) return;
      this.observe(reply);
      queueMicrotask(() => this.listener?.(reply));
    });
  }
  post(request: StoreRequest): void { this.engine.handle(structuredClone(request)); }
  onReply(listener: (reply: StoreReply) => void): void { this.listener = listener; }
  onRestart(listener: () => void): void { this.restartListener = listener; }
  waitSync(): boolean { throw new Error('not modelled'); }
  restart() {
    this.disk.dead.add(this.generation);
    this.disk.gates = this.disk.gates.filter(gate => gate.engine !== this.generation);
    this.start();
    this.restartListener?.();
  }
}

const settle = async () => { for (let i = 0; i < 4; i++) await new Promise(resolve => setImmediate(resolve)); };

type Choice = string;
type Script = Array<'S' | 'P'>;

/** Runs one schedule: `choices` first, then `pick` at each later decision. Returns the options it saw. */
/* Checkpoint ids and temporary names come from per-engine counters; equal states reached by different paths get equal keys. */
const canonical = (text: string) => { const names = new Map<string, string>(); return text.replace(/g\d+s\d+/g, name => { if (!names.has(name)) names.set(name, `c${names.size}`); return names.get(name)!; }); };

async function run(script: Script, choices: Choice[], faults: number, seen?: Set<string>): Promise<{ options: Choice[][]; violations: string[] }> {
  const docs: string[] = [];
  let doc: any = { id: 'P', name: 'P', f: {}, edits: [] };
  docs.push(JSON.stringify(doc));
  const disk = new ModelDisk({ [SLOT]: JSON.stringify({ cp: 'a', seq: 1, v: '1', proj: doc }) });
  /** Post id of each patch (its version) and each slot (the version it holds). */
  const posted = new Map<number, { kind: 'S' | 'P'; version: number }>();
  /** The version acknowledged durable, at the disk state it was acknowledged in. */
  const acks: Array<{ state: number; version: number }> = [];
  let lastPostId = 0;
  const channel = new ModelChannel(disk, reply => {
    if (reply.type !== 'written') return;
    const op = posted.get(reply.id);
    if (op) acks.push({ state: disk.states.length - 1, version: op.version });
  });
  const store = createStore(DIR, { thread: { storeChannel: () => channel }, writeDelayMs: 0 });
  // The engine numbers every request; map them to the renderer's posts.
  const post = channel.post.bind(channel);
  channel.post = (request: StoreRequest) => { lastPostId = request.id; post(request); };
  await store.load();
  await settle();

  const violations: string[] = [];
  let newest = -1, checked = 0;
  const check = () => {
    for (; checked < disk.states.length; checked++) {
      const state = disk.states[checked]!, index = checked;
      const slot = state.slot === null ? null : JSON.parse(state.slot);
      const recovered = slot ? JSON.stringify(recoverProject(slot, state.journal === null ? null : JSON.parse(state.journal))) : null;
      const at = recovered === null ? -1 : docs.indexOf(recovered);
      const acked = Math.max(0, ...acks.filter(ack => ack.state <= index).map(ack => ack.version));
      if (recovered === null) violations.push(`state ${index}: no slot`);
      else if (at < 0) violations.push(`state ${index}: a document that never existed ${recovered}`);
      else {
        if (at < newest) violations.push(`state ${index}: older (${at}) than an earlier crash point (${newest})`);
        if (at < acked) violations.push(`state ${index}: lost acknowledged patch ${acked} (recovered ${at})`);
        newest = Math.max(newest, at);
      }
    }
  };
  let next = 0, version = 0, budget = faults;
  const options: Choice[][] = [];
  const asVersion = (id: unknown) => typeof id === 'number' ? (posted.get(id)?.version ?? 'r') : id;
  const key = () => {
    const engine: any = channel.engine.inspectProject('P');
    if (engine) {
      engine.acks = engine.acks.map(asVersion);
      if (engine.slot) engine.slot.id = asVersion(engine.slot.id);
      if (engine.writingSlot) engine.writingSlot.id = asVersion(engine.writingSlot.id);
      if (engine.writingAcks) engine.writingAcks = engine.writingAcks.map(asVersion);
    }
    const internal: any = store;
    return canonical(JSON.stringify({
      next, version, budget, newest, acked: Math.max(0, ...acks.map(ack => ack.version)),
      disk: [disk.files.get(SLOT) ?? null, disk.files.get(JOURNAL) ?? null],
      gates: disk.gates.map(gate => `${gate.kind}:${gate.file}:${gate.payload}`).sort(),
      engine,
      // What a replacement engine would be given (only while one can still happen).
      unwritten: budget ? [...internal.unwritten].sort((a: any, b: any) => a[1] - b[1]).map(([name, id]: any) => [name, asVersion(id), internal.committed.get(name)?.toString('utf8') ?? null]) : null,
    }));
  };
  for (let step = 0; ; step++) {
    const here: Choice[] = [];
    if (next < script.length) {
      here.push('post');
      if (budget && script[next] === 'S') here.push('refuse');
    }
    disk.gates.forEach((_, index) => { here.push(`ok:${index}`); if (budget) here.push(`fail:${index}`); });
    if (budget && (disk.gates.length || step > 0)) here.push('restart');
    if (!here.length) break;
    check();
    if (seen && step >= choices.length) {
      const state = key();
      if (seen.has(state)) break;
      seen.add(state);
    }
    options.push(here);
    const choice = choices[step] ?? here[0]!;
    if (!here.includes(choice)) throw new Error(`schedule diverged at ${step}: ${choice} not in ${here}`);
    if (choice === 'post' || choice === 'refuse') {
      const kind = script[next++]!;
      if (choice === 'refuse') budget--;
      if (kind === 'P') {
        version++;
        const patches = [{ path: ['edits'], exists: true, value: [...doc.edits, version] }, { path: ['f', String(version % 2)], exists: true, value: version }];
        doc = { ...doc, edits: [...doc.edits, version], f: { ...doc.f, [version % 2]: version } };
        docs.push(JSON.stringify(doc));
        void store.appendJournal!('projectJournal.P', JSON.stringify({ revision: version, patches })).catch(() => {});
        posted.set(lastPostId, { kind, version });
      } else {
        void Promise.resolve(store.setSerialized!('project.P', choice === 'refuse' ? '{refused' : JSON.stringify({ v: '1', proj: doc }))).catch(() => {});
        if (choice !== 'refuse') posted.set(lastPostId, { kind, version });
      }
    } else if (choice === 'restart') {
      budget--;
      channel.restart();
    } else {
      const [verdict, index] = choice.split(':');
      const gate = disk.gates.splice(Number(index), 1)[0]!;
      if (verdict === 'fail') { budget--; gate.settle(Object.assign(new Error('ENOSPC: no space left on device'), { code: 'ENOSPC' })); }
      else { try { gate.apply(); gate.settle(); } catch (error) { gate.settle(error as Error); } }
    }
    await settle();
  }

  check();
  return { options, violations };
}

/** Every schedule of `script` with at most `faults` injected faults. */
async function explore(script: Script, faults: number, memo = true): Promise<{ schedules: number; violations: string[] }> {
  let schedules = 0;
  const violations: string[] = [];
  const stack: Choice[][] = [[]];
  const seen = memo ? new Set<string>() : undefined;
  while (stack.length) {
    const prefix = stack.pop()!;
    const result = await run(script, prefix, faults, seen);
    schedules++;
    for (const violation of result.violations) violations.push(`${script.join('')} [${prefix.join(' ')}] ${violation}`);
    if (violations.length > 5) break;
    // Branch on every decision after the prefix (the run took the first option there).
    for (let step = result.options.length - 1; step >= prefix.length; step--) {
      const taken = Array.from({ length: step }, (_, index) => prefix[index] ?? result.options[index]![0]!);
      for (const alternative of result.options[step]!.slice(1)) stack.push([...taken, alternative]);
    }
  }
  return { schedules, violations };
}

/** Every arrangement of `slots` slot writes and `patches` patches. */
function scripts(slots: number, patches: number): Script[] {
  const out: Script[] = [];
  const build = (script: Script, s: number, p: number) => {
    if (!s && !p) { out.push(script); return; }
    if (s) build([...script, 'S'], s - 1, p);
    if (p) build([...script, 'P'], s, p - 1);
  };
  build([], slots, patches);
  return out;
}

describe('SAVE-1 model check', () => {
  it.skipIf(!process.env.PM_MODEL_ONE)('one script', { timeout: 600_000 }, async () => {
    const began = Date.now();
    const result = await explore(process.env.PM_MODEL_ONE!.split('') as Script, Number(process.env.PM_MODEL_FAULTS ?? 1), process.env.PM_MODEL_MEMO !== '0');
    console.log(`MODEL ${process.env.PM_MODEL_ONE}: ${result.schedules} schedules in ${Date.now() - began} ms; ${result.violations.slice(0, 3).join(' | ')}`);
  });

  it('a sanity schedule: patches and a slot write, no fault', async () => {
    const result = await run(['P', 'S', 'P'], [], 0);
    expect(result.violations).toEqual([]);
  });

  /* 35 arrangements of the script; states reached twice are explored once
     (the key holds everything that decides what follows, and what the check
     has seen so far). Mutations of the protocol it catches: a failed slot
     write that shrinks the journal, recovery that ignores sequence numbers,
     a journal shrunk before its slot is on disk, a replacement engine that
     does not read the journal on disk first. */
  it('every interleaving of 3 slot writes and 4 patches, with any one fault or worker restart, recovers a real state with every acknowledged patch', { timeout: 1_800_000 }, async () => {
    let schedules = 0;
    for (const script of scripts(3, 4)) {
      const result = await explore(script, 1);
      schedules += result.schedules;
      expect(result.violations, script.join('')).toEqual([]);
    }
    console.log(`SAVE-1 model check (3 slot writes, 4 patches, 1 fault): ${schedules} schedules`);
  });
});
