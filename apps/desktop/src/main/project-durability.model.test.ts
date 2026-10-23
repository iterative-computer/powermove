/*
 * SAVE-1 model check. The real FileStore (storage.ts), StoreEngine and
 * ProjectDurability run against an in-memory file system on which every
 * operation that changes what a restart reads (a file renamed into place, or
 * unlinked) waits for the checker. A renderer posts a script of slot writes
 * and journal patches; the checker explores, depth first by re-execution,
 * every order in which those posts and the pending disk operations can
 * happen, with one injected fault: a disk operation that fails, a slot write
 * or a journal entry the engine refuses (the window keeps the edit in its
 * document), an entry main refuses for its size before posting it, the
 * storage worker replaced (its operations abandoned before or after they
 * landed), an entry lost in flight as the worker stops, or a replacement
 * worker that fails to read the project once.
 *
 * Every disk state any schedule passes through is a crash point. Each must
 * recover (shared/project-journal.ts recoverProject):
 * - a state the document actually had (after some number of its patches);
 * - never fewer patches than an earlier crash point in the same schedule;
 * - every patch the engine had acknowledged as written by then.
 * And once every write has settled (quiescence), disk recovers the final
 * document, or main's failing keys name the project; then (liveness) one more
 * fault-free checkpoint and edit must be accepted, recover the final
 * document, and leave nothing failing. The worker's replies can be held while
 * later posts go out, and a second window (not the project's owner) can try
 * to write the project from its stored copy.
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
  /** The next read of a project file by this engine generation fails (a replacement worker's read fault). */
  failRead = 0;
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
      readFile: ((file: string) => live(() => {
        if (this.failRead === engine && visible(String(file))) { this.failRead = 0; throw Object.assign(new Error('EIO: i/o error, read'), { code: 'EIO' }); }
        const data = this.files.get(String(file)); if (data === undefined) throw missing(String(file)); return data;
      })) as any,
      writeFile: ((file: string, data: any) => live(() => { this.files.set(String(file), typeof data === 'string' ? data : Buffer.from(data).toString('utf8')); })) as any,
      open: ((file: string) => live(() => {
        if (String(file) === DIR) return { sync: async () => {}, close: async () => {} };
        if (this.failRead === engine && visible(String(file))) { this.failRead = 0; throw Object.assign(new Error('EIO: i/o error, read'), { code: 'EIO' }); }
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
  /** Replies held back (a busy port), delivered in order on release. */
  held: StoreReply[] | null = null;
  release() { const queue = this.held ?? []; this.held = null; for (const reply of queue) this.listener?.(reply); }
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
      if (this.held) this.held.push(reply);
      else queueMicrotask(() => this.listener?.(reply));
    });
  }
  /** Posts are dropped (a worker that stopped before handling them). */
  dropping = false;
  post(request: StoreRequest): void { if (!this.dropping) this.engine.handle(structuredClone(request)); }
  onReply(listener: (reply: StoreReply) => void): void { this.listener = listener; }
  onRestart(listener: () => void): void { this.restartListener = listener; }
  waitSync(): boolean { throw new Error('not modelled'); }
  restart(options: { failRead?: boolean } = {}) {
    // A stopped worker's replies that were held never arrive.
    this.held = this.held ? [] : null;
    this.disk.dead.add(this.generation);
    this.disk.gates = this.disk.gates.filter(gate => gate.engine !== this.generation);
    this.dropping = false;
    this.start();
    if (options.failRead) this.disk.failRead = this.generation;
    this.restartListener?.();
  }
}

const settle = async () => { for (let i = 0; i < 4; i++) await new Promise(resolve => setImmediate(resolve)); };

type Choice = string;
type Script = Array<'S' | 'P'>;

/** Runs one schedule: `choices` first, then `pick` at each later decision. Returns the options it saw. */
/* Checkpoint ids and temporary names come from per-engine counters; equal states reached by different paths get equal keys. */
const canonical = (text: string) => { const names = new Map<string, string>(); return text.replace(/g\d+s\d+/g, name => { if (!names.has(name)) names.set(name, `c${names.size}`); return names.get(name)!; }); };

/** The fault kinds before SAVE-1c's held replies and second window (the long sweep keeps to these). */
const BASIC = new Set(['refuse', 'refuse-two', 'oversize', 'lose', 'fail', 'restart', 'restart-read-fails']);

async function run(script: Script, choices: Choice[], faults: number, seen?: Set<string>, trace = false, kinds?: Set<string>): Promise<{ options: Choice[][]; violations: string[] }> {
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
  // A small limit for one entry, so an entry over main's own size check fits the model.
  const store = createStore(DIR, { thread: { storeChannel: () => channel }, writeDelayMs: 0, journalEntryBytes: 4096 });
  // Window A has the project; window B only sees its stored copy (SAVE-1c).
  store.setOwnership!(() => 'A');
  // The engine numbers every request; map them to the renderer's posts.
  const post = channel.post.bind(channel);
  channel.post = (request: StoreRequest) => { if (request.op === 'append' || request.op === 'setSerialized') lastPostId = request.id; post(request); };
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
      failing: [...(internal.failing?.keys?.() ?? [])].sort(),
      // Main's closed journals and replays (request ids: deterministic per path, so equal keys stay sound).
      closed: [...(internal.closedJournals ?? [])], replayed: [...(internal.replayed ?? [])], appends: [...(internal.appends ?? [])],
      failRead: disk.failRead === channel.generation,
      held: channel.held ? JSON.stringify(channel.held) : null,
    }));
  };
  for (let step = 0; ; step++) {
    const here: Choice[] = [];
    if (next < script.length) {
      here.push('post');
      // The next two posted in one task: the second is on its way before any reply to the first.
      if (next + 1 < script.length) here.push('post-two');
      if (budget) here.push('refuse');
      // A refused entry with the next operation already on its way behind it.
      if (budget && next + 1 < script.length) here.push('refuse-two');
      // A refused entry whose reply arrives late: replies are held from it on, while later posts go out.
      if (budget && next + 1 < script.length && !channel.held) here.push('refuse-late');
      if (budget && script[next] === 'P') here.push('oversize', 'lose');
    }
    disk.gates.forEach((_, index) => { here.push(`ok:${index}`); if (budget) here.push(`fail:${index}`); });
    if (budget && (disk.gates.length || step > 0)) here.push('restart', 'restart-read-fails');
    // Replies held while later posts go out (a fault), and released.
    if (channel.held) here.push('release'); else if (budget && next < script.length) here.push('hold');
    // Window B writes the project from its stored copy (a rename there).
    if (budget && step > 0) here.push('stray');
    // A narrower sweep leaves some fault kinds out (`fail:<n>` counts as 'fail').
    if (kinds) for (let index = here.length - 1; index >= 0; index--) {
      const kind = here[index]!.split(':')[0]!;
      if (kind !== 'post' && kind !== 'post-two' && kind !== 'ok' && kind !== 'release' && !kinds.has(kind)) here.splice(index, 1);
    }
    if (!here.length) break;
    check();
    if (seen && step >= choices.length) {
      const state = key();
      if (seen.has(state)) break;
      seen.add(state);
    }
    options.push(here);
    const choice = choices[step] ?? here[0]!;
    if (trace) console.log('STEP', step, choice, 'of', here.join(' '));
    if (!here.includes(choice)) throw new Error(`schedule diverged at ${step}: ${choice} not in ${here}`);
    const posts = choice === 'post-two' || choice === 'refuse-two' ? 2 : 1;
    if (choice === 'refuse-late') channel.held = [];
    if (['post', 'post-two', 'refuse', 'refuse-two', 'refuse-late', 'oversize', 'lose'].includes(choice)) for (let n = 0; n < posts; n++) {
      const kind = script[next++]!;
      // In a burst only the first is the faulty one.
      const fault = n === 0 && choice !== 'post' && choice !== 'post-two' ? (choice === 'refuse-two' || choice === 'refuse-late' ? 'refuse' : choice) : 'post';
      if (fault !== 'post') budget--;
      if (kind === 'P') {
        version++;
        // An oversize edit carries a value past main's entry limit; the window keeps it in its document.
        const value = fault === 'oversize' ? 'x'.repeat(5000) : version;
        const patches = [{ path: ['edits'], exists: true, value: [...doc.edits, version] }, { path: ['f', String(version % 2)], exists: true, value }];
        doc = { ...doc, edits: [...doc.edits, version], f: { ...doc.f, [version % 2]: value } };
        docs.push(JSON.stringify(doc));
        const before = lastPostId;
        if (fault === 'lose') channel.dropping = true;
        void store.appendJournal!('projectJournal.P', fault === 'refuse' ? '{refused' : JSON.stringify({ revision: version, patches }), 'A').catch(() => {});
        if (fault === 'post' && lastPostId !== before) posted.set(lastPostId, { kind, version });
        // The worker stops with the entry unhandled, and is replaced.
        if (fault === 'lose') channel.restart();
      } else {
        void Promise.resolve(store.setSerialized!('project.P', fault === 'refuse' ? '{refused' : JSON.stringify({ v: '1', proj: doc }), 'A')).catch(() => {});
        if (fault === 'post') posted.set(lastPostId, { kind, version });
      }
    } else if (choice === 'hold') {
      budget--;
      channel.held = [];
    } else if (choice === 'release') {
      channel.release();
    } else if (choice === 'stray') {
      budget--;
      // B's view: what main had committed (possibly behind replies still on their way).
      const stored = (store as any).committed.get('project.P')?.toString('utf8');
      const journal = (store as any).committed.get('projectJournal.P')?.toString('utf8');
      const copy = stored ? recoverProject(JSON.parse(stored), journal ? JSON.parse(journal) : []) : null;
      if (copy) void Promise.resolve(store.setSerialized!('project.P', JSON.stringify({ v: '1', proj: { ...copy, name: 'renamed elsewhere' } }), 'B')).catch(() => {});
    } else if (choice === 'restart' || choice === 'restart-read-fails') {
      budget--;
      channel.restart({ failRead: choice === 'restart-read-fails' });
    } else {
      const [verdict, index] = choice.split(':');
      const gate = disk.gates.splice(Number(index), 1)[0]!;
      if (verdict === 'fail') { budget--; gate.settle(Object.assign(new Error('ENOSPC: no space left on device'), { code: 'ENOSPC' })); }
      else { try { gate.apply(); gate.settle(); } catch (error) { gate.settle(error as Error); } }
    }
    await settle();
  }

  check();

  // Quiescence: every write settles (no more faults); disk then holds the final document, or main says it does not.
  channel.release();
  const drain = async () => { for (let round = 0; round < 50; round++) {
    let flushed = false;
    void store.flushAll().then(() => { flushed = true; });
    await settle();
    while (disk.gates.length) {
      const gate = disk.gates.shift()!;
      try { gate.apply(); gate.settle(); } catch (error) { gate.settle(error as Error); }
      await settle();
    }
    if (flushed && !disk.gates.length) break;
  } };
  await drain();
  check();
  if (trace) disk.states.forEach((state, index) => console.log('STATE', index, state.slot?.slice(0, 60), state.journal?.slice(0, 300)));
  const last = disk.states.at(-1)!;
  const final = last.slot === null ? null : JSON.stringify(recoverProject(JSON.parse(last.slot), last.journal === null ? null : JSON.parse(last.journal)));
  const failing = ((store as any).failingKeys?.() ?? []) as string[];
  if (final !== docs.at(-1) && !failing.some(key => key === 'project.P' || key === 'projectJournal.P')) {
    violations.push(`quiescent: disk recovers ${final === null ? 'nothing' : `version ${docs.indexOf(final)}`} of ${docs.length - 1}, and main reports nothing failing`);
  }

  // Liveness: a fault-free checkpoint, then an edit. It must be accepted, and disk must recover the final document.
  disk.failRead = 0; // an injected fault still waiting to fire is not part of the fault-free tail
  await Promise.resolve(store.setSerialized!('project.P', JSON.stringify({ v: '1', proj: doc }), 'A')).catch(() => {});
  await drain();
  version++;
  doc = { ...doc, edits: [...doc.edits, version] };
  docs.push(JSON.stringify(doc));
  const tail = await store.appendJournal!('projectJournal.P', JSON.stringify({ revision: version, patches: [{ path: ['edits'], exists: true, value: doc.edits }] }), 'A').then(() => 'accepted', (error: Error) => error.message);
  await drain();
  check();
  const tailState = disk.states.at(-1)!;
  const recoveredTail = tailState.slot === null ? null : JSON.stringify(recoverProject(JSON.parse(tailState.slot), tailState.journal === null ? null : JSON.parse(tailState.journal)));
  const stillFailing = ((store as any).failingKeys?.() ?? []) as string[];
  if (tail !== 'accepted') violations.push(`liveness: an edit after a fault-free checkpoint was refused (${tail})`);
  if (recoveredTail !== docs.at(-1)) violations.push(`liveness: after a fault-free checkpoint and edit disk recovers ${recoveredTail === null ? 'nothing' : `version ${docs.indexOf(recoveredTail)}`} of ${docs.length - 1}`);
  if (stillFailing.length) violations.push(`liveness: still failing after a fault-free checkpoint and edit: ${stillFailing.join(', ')}`);
  return { options, violations };
}

/** Every schedule of `script` with at most `faults` injected faults. */
async function explore(script: Script, faults: number, memo = true, kinds?: Set<string>): Promise<{ schedules: number; violations: string[] }> {
  let schedules = 0;
  const violations: string[] = [];
  const stack: Choice[][] = [[]];
  const seen = memo ? new Set<string>() : undefined;
  while (stack.length) {
    const prefix = stack.pop()!;
    const result = await run(script, prefix, faults, seen, false, kinds);
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
  it.skipIf(!process.env.PM_MODEL_ONE)('one script', { timeout: 3_600_000 }, async () => {
    const began = Date.now();
    const result = await explore(process.env.PM_MODEL_ONE!.split('') as Script, Number(process.env.PM_MODEL_FAULTS ?? 1), process.env.PM_MODEL_MEMO !== '0', process.env.PM_MODEL_BASIC ? BASIC : undefined);
    console.log(`MODEL ${process.env.PM_MODEL_ONE}: ${result.schedules} schedules in ${Date.now() - began} ms; ${result.violations.slice(0, 3).join(' | ')}`);
  });

  it.skipIf(!process.env.PM_MODEL_SCHEDULE)('one schedule', async () => {
    const spec = process.env.PM_MODEL_SCHEDULE!, at = spec.indexOf(':');
    const script = at < 0 ? spec : spec.slice(0, at), choices = at < 0 ? '' : spec.slice(at + 1);
    const result = await run(script!.split('') as Script, choices ? choices.split(',') : [], Number(process.env.PM_MODEL_FAULTS ?? 1), undefined, true);
    console.log('SCHEDULE', JSON.stringify(result.violations, null, 1));
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
     does not read the journal on disk first, an entry accepted after a
     refused one, a journal reopened by a slot older than the refusal, a
     journal write silently skipped (quiescence), and (SAVE-1b) a journal left
     open after an entry main refused for its size, one lost as the worker
     stopped, one the engine refused or could not read the project for (with
     the next entry already on its way), or a replayed slot judged by its new
     request id. */
  it('every interleaving of 3 slot writes and 4 patches, with any one fault of the earlier kinds, recovers a real state with every acknowledged patch', { timeout: 3_600_000 }, async () => {
    let schedules = 0;
    for (const script of scripts(3, 4)) {
      const result = await explore(script, 1, true, BASIC);
      schedules += result.schedules;
      expect(result.violations, script.join('')).toEqual([]);
    }
    console.log(`SAVE-1 model check (3 slot writes, 4 patches, 1 fault of the earlier kinds): ${schedules} schedules`);
    expect(schedules).toBeGreaterThan(100_000);
  });

  /* Every fault kind: held replies, a refusal whose reply arrives late, a second window writing its stored copy. */
  it('every interleaving of 2 slot writes and 3 patches, with any one fault, recovers a real state and stays live', { timeout: 3_600_000 }, async () => {
    let schedules = 0;
    for (const script of scripts(2, 3)) {
      const result = await explore(script, 1);
      schedules += result.schedules;
      expect(result.violations, script.join('')).toEqual([]);
    }
    console.log(`SAVE-1 model check (2 slot writes, 3 patches, 1 fault of any kind): ${schedules} schedules`);
    expect(schedules).toBeGreaterThan(50_000);
  });

  /* Two faults of any kind: a late refusal then main's own refusal (the reviewer's R5a), a hold then a replacement that cannot read, and so on. */
  it('every interleaving of 2 slot writes and 2 patches, or 1 slot write and 3 patches, with any two faults, recovers a real state and stays live', { timeout: 3_600_000 }, async () => {
    let schedules = 0;
    for (const script of [...scripts(2, 2), ...scripts(1, 3)]) {
      const result = await explore(script, 2);
      schedules += result.schedules;
      expect(result.violations, script.join('')).toEqual([]);
    }
    console.log(`SAVE-1 model check (2 slots x 2 patches and 1 slot x 3 patches, 2 faults of any kind): ${schedules} schedules`);
    expect(schedules).toBeGreaterThan(50_000);
  });
});
