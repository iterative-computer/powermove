import { createHash, randomUUID } from 'node:crypto';
import { copyFile, link, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';

type Reply = { ok: boolean; code?: string; error?: string; value?: any; special?: boolean };
type Request = { op: string; args?: any };
/** `hash` is the tree's canonical hash ('' until a text checkpoint's first
 *  load answers it). A checkpoint main received as JSON text in any member
 *  order (document-kernel commit) also keeps `sourceHash`, its bytes' hash,
 *  and is loaded as text. */
type Checkpoint = { file: string; document: string; revision: number; hash: string; preparedRevision: number; sourceHash?: string };
type Intern = { request: string; native: number };
type SavedCall = { request: string; file?: string; jsonFile?: string };
type Owner = {
  dir: string; generation: number; tail: Promise<void>; base?: Checkpoint; prepared?: Checkpoint;
  patches: Array<{ request: string; file: string; bytes: number }>; patchBytes: number;
  context: Map<string, SavedCall>; interns: Map<number, Intern>; nativeIds: Map<number, number>; nextId: number;
};
const MAX_PATCHES = 128, MAX_PATCH_BYTES = 8 * 1024 * 1024;
const pause = () => new Promise(resolve => setTimeout(resolve, 10));

/** Recovery never needs a renderer task: even a page blocked in sendSync can
 * wait for the native evaluator. Only immutable checkpoint files and bounded
 * patch suffixes survive in main; neither a second document nor font buffers
 * are retained in its heap. */
export class DocumentRecovery {
  private owners = new Map<number, Promise<Owner>>();
  private stopped = false;
  constructor(private readonly options: {
    cache: string;
    ready(deadline: number, after?: number): Promise<number>;
    call(owner: number, request: string, bytes?: Uint8Array, flush?: boolean): Promise<string>;
  }) {}

  private owner(id: number): Promise<Owner> {
    let pending = this.owners.get(id);
    if (!pending) {
      pending = (async () => {
        await mkdir(this.options.cache, { recursive: true, mode: 0o700 });
        const dir = await mkdtemp(path.join(this.options.cache, 'owner-'));
        return { dir, generation: 0, tail: Promise.resolve(), patches: [], patchBytes: 0, context: new Map(), interns: new Map(), nativeIds: new Map(), nextId: 0 };
      })();
      this.owners.set(id, pending);
      void pending.catch(() => { if (this.owners.get(id) === pending) this.owners.delete(id); });
    }
    return pending;
  }
  call(id: number, request: string, bytes?: Uint8Array, flush = false): Promise<string> {
    if (this.stopped) return Promise.reject(new Error('The document engine is stopping'));
    return this.owner(id).then(state => {
      const job = state.tail.then(() => this.run(id, state, JSON.parse(request), bytes, flush));
      // Do not retain a successful, possibly large JSON answer in this queue.
      state.tail = job.then(() => {}, () => {});
      return job;
    });
  }
  private async raw(id: number, request: Request, bytes?: Uint8Array, flush = false): Promise<Reply> {
    const reply: Reply = JSON.parse(await this.options.call(id, JSON.stringify(request), bytes, flush));
    if (!reply.ok && reply.code === 'unavailable') throw Object.assign(new Error(reply.error), { code: 'unavailable' });
    return reply;
  }
  private async checked(id: number, request: Request, deadline: number, bytes?: Uint8Array): Promise<any> {
    for (;;) {
      const reply = await this.raw(id, request, bytes);
      if (reply.ok) return reply.value;
      if (reply.code !== 'busy' || Date.now() >= deadline) throw new Error(reply.error || 'Native document recovery failed');
      await pause();
    }
  }
  private async load(id: number, checkpoint: Checkpoint, deadline: number): Promise<void> {
    const text = checkpoint.sourceHash !== undefined;
    let receipt = await this.checked(id, { op: 'load', args: { path: checkpoint.file, document: checkpoint.document, revision: checkpoint.preparedRevision,
      hash: text ? checkpoint.sourceHash : checkpoint.hash, ...(text ? { source: 'text' } : {}) } }, deadline);
    while (receipt.pending) {
      if (Date.now() >= deadline) throw new Error('Native document recovery timed out');
      await pause();
      receipt = await this.checked(id, { op: 'poll' }, deadline);
    }
    if (text && !checkpoint.hash && typeof receipt.hash === 'string') checkpoint.hash = receipt.hash;
    if (receipt.revision !== checkpoint.preparedRevision || receipt.hash !== checkpoint.hash) throw new Error('Native recovery checkpoint did not match');
  }
  private async restore(id: number, state: Owner, generation: number, deadline: number): Promise<void> {
    if (state.base) {
      await this.load(id, state.base, deadline);
      await this.checked(id, { op: 'activate', args: { revision: state.base.revision, preparedRevision: state.base.preparedRevision, hash: state.base.hash } }, deadline);
      for (const patch of state.patches) {
        const result = await this.checked(id, JSON.parse(patch.request), deadline, await readFile(patch.file));
        if (result.code !== 0) throw new Error('Native recovery patch suffix did not match');
      }
    }
    for (const saved of state.context.values()) await this.checked(id, JSON.parse(saved.jsonFile ? await readFile(saved.jsonFile, 'utf8') : saved.request), deadline, saved.file ? await readFile(saved.file) : undefined);
    state.nativeIds.clear();
    for (const [virtual, intern] of state.interns) { intern.native = await this.checked(id, JSON.parse(intern.request), deadline); state.nativeIds.set(intern.native, virtual); }
    if (state.prepared) await this.load(id, state.prepared, deadline);
    state.generation = generation;
  }
  private async save(state: Owner, request: Request, bytes?: Uint8Array, diskRequest = false): Promise<SavedCall> {
    const saved: SavedCall = { request: JSON.stringify(request) };
    if (diskRequest) { saved.jsonFile = path.join(state.dir, randomUUID()); await writeFile(saved.jsonFile, saved.request, { mode: 0o600 }); saved.request = ''; }
    if (bytes) { saved.file = path.join(state.dir, randomUUID()); await writeFile(saved.file, bytes, { mode: 0o600 }); }
    return saved;
  }
  private async remove(saved: { file?: string; jsonFile?: string } | undefined): Promise<void> { await Promise.all([saved?.file, saved?.jsonFile].filter((file): file is string => !!file).map(file => rm(file, { force: true }))); }
  private async resetPatches(state: Owner): Promise<void> {
    const old = state.patches; state.patches = []; state.patchBytes = 0;
    await Promise.all(old.map(saved => this.remove(saved)));
  }
  private async compact(id: number, state: Owner, deadline: number): Promise<void> {
    if (!state.base || !state.patches.length) return;
    const revision = JSON.parse(state.patches.at(-1)!.request).args.revision;
    const file = path.join(state.dir, randomUUID());
    let accepted = false;
    try {
      await this.checked(id, { op: 'persist', args: { revision, path: file } }, deadline);
      let receipt: any;
      do {
        if (Date.now() >= deadline) throw new Error('Native checkpoint persistence timed out');
        await pause();
        receipt = await this.checked(id, { op: 'pollPersist', args: { revision } }, deadline);
      } while (receipt.pending);
      if (receipt.revision !== revision) throw new Error('Native checkpoint persistence revision changed');
      await this.checked(id, { op: 'acceptPersist', args: { revision, hash: receipt.hash } }, deadline);
      const old = state.base;
      // The utility persisted canonical bytes: no longer a text checkpoint.
      const { sourceHash: _text, ...previous } = old ?? {} as Partial<Checkpoint>;
      state.base = { ...previous as Checkpoint, file, revision, preparedRevision: revision, hash: receipt.hash };
      accepted = true;
      await this.resetPatches(state);
      await this.remove(old);
    } finally { if (!accepted) await rm(file, { force: true }); }
  }
  private translate(state: Owner, request: Request): Request {
    if (['ev', 'expressionError', 'release'].includes(request.op)) return { ...request, args: { ...request.args, id: state.interns.get(request.args.id)?.native ?? 0xffffffff } };
    if (request.op === 'evBatch') return { ...request, args: { ...request.args, ids: request.args.ids.map((id: number) => state.interns.get(id)?.native ?? 0xffffffff) } };
    return request;
  }
  private async run(id: number, state: Owner, request: Request, bytes?: Uint8Array, flush = false): Promise<string> {
    const deadline = Date.now() + 120_000;
    let after: number | undefined;
    for (;;) {
      const generation = await this.options.ready(deadline, after);
      try {
        if (state.generation !== generation) await this.restore(id, state, generation, deadline);
        if (request.op === 'load') {
          // IPC staging is removed as soon as commit finishes; this private
          // copy is the only recovery source and is removed on replacement.
          // The staging file is complete and never written again, so a hard
          // link is that copy (copying the real project's 180 MB held its
          // ingestion up 0.6 to 0.8 s); a copy across volumes.
          const file = path.join(state.dir, randomUUID());
          await link(request.args.path, file).catch(() => copyFile(request.args.path, file));
          const text = request.args.source === 'text';
          const prepared: Checkpoint = { file, document: request.args.document, revision: request.args.revision, preparedRevision: request.args.revision,
            hash: text ? '' : request.args.hash, ...(text ? { sourceHash: request.args.hash } : {}) };
          const previous = state.prepared;
          state.prepared = prepared;
          await this.remove(previous);
          await this.load(id, prepared, deadline);
          return JSON.stringify(await this.raw(id, { op: 'poll' }));
        }
        if (request.op === 'apply' && (state.patches.length >= MAX_PATCHES || state.patchBytes + (bytes?.byteLength ?? 0) > MAX_PATCH_BYTES)) await this.compact(id, state, deadline);
        const reply = await this.raw(id, this.translate(state, request), bytes, flush);
        if (!reply.ok) return JSON.stringify(reply);
        if (request.op === 'activate') {
          if (!state.prepared) throw new Error('The native document recovery source is missing');
          const old = state.base;
          state.base = { ...state.prepared, revision: request.args.revision };
          state.prepared = undefined;
          await this.resetPatches(state); await this.remove(old);
        } else if (request.op === 'resync') {
          const file = path.join(state.dir, randomUUID());
          await writeFile(file, request.args.json, { mode: 0o600 });
          const old = state.base;
          /* The barrier's resync JSON is the canonical project form in
             JSON.stringify's member order, not canonical bytes: a text
             checkpoint, checked against its bytes' hash and then known by
             the canonical hash its first load answers (as a text open). */
          state.base = { file, document: request.args.document || old?.document || 'untitled', revision: request.args.revision, preparedRevision: request.args.revision,
            hash: '', sourceHash: createHash('sha256').update(request.args.json).digest('hex') };
          await this.resetPatches(state); await this.remove(old);
        } else if (request.op === 'apply' && reply.value.code === 0) {
          const saved = await this.save(state, request, bytes);
          state.patches.push({ ...saved, file: saved.file!, bytes: bytes?.byteLength ?? 0 }); state.patchBytes += bytes?.byteLength ?? 0;
        } else if (request.op === 'intern' && reply.value >= 0) {
          const found = state.nativeIds.get(reply.value);
          const virtual = found ?? state.nextId++;
          if (found === undefined) { state.interns.set(virtual, { request: JSON.stringify(request), native: reply.value }); state.nativeIds.set(reply.value, virtual); }
          reply.value = virtual;
        } else if (request.op === 'release') { const entry = state.interns.get(request.args.id); if (entry) state.nativeIds.delete(entry.native); state.interns.delete(request.args.id); }
        else if (['define', 'setFontSeeds', 'fontOutlines', 'addFont', 'addFontAnswers'].includes(request.op)) {
          const saveContext = async (key: string, command: Request, binary?: Uint8Array) => {
            const saved = await this.save(state, command, binary, true);
            const old = state.context.get(key); state.context.set(key, saved); await this.remove(old);
          };
          if (request.op === 'addFontAnswers') {
            // Match Rust Answers::merge: first face for an id wins, latest
            // answer for an identical query wins. Repeated text queries must
            // not accumulate a historical replay log or duplicate JSON heap.
            const answers = JSON.parse(request.args.json);
            for (const face of answers.faces ?? []) {
              const key = `face:${face.id}`;
              if (!state.context.has(key)) await saveContext(key, { op: 'addFontAnswers', args: { json: JSON.stringify({ faces: [face], answers: [] }) } });
            }
            for (const answer of answers.answers ?? []) {
              const query = Object.fromEntries(Object.entries(answer.query).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0));
              const key = `answer:${createHash('sha256').update(JSON.stringify(query)).digest('hex')}`;
              await saveContext(key, { op: 'addFontAnswers', args: { json: JSON.stringify({ faces: [], answers: [answer] }) } });
            }
          } else {
            const key = request.op === 'define' ? `define:${request.args.id}` : request.op === 'addFont' ? `font:${request.args.blob}:${request.args.index}` : request.op;
            await saveContext(key, request, bytes);
          }
        } else if (request.op === 'clearFonts') {
          for (const [key, saved] of state.context) if (key.startsWith('font:') || key.startsWith('face:') || key.startsWith('answer:')) { state.context.delete(key); await this.remove(saved); }
          const key = 'clearFonts'; state.context.delete(key); state.context.set(key, { request: JSON.stringify(request) });
        } else if (request.op === 'cancel') { await this.remove(state.prepared); state.prepared = undefined; }
        else if (request.op === 'close') {
          this.owners.delete(id); await rm(state.dir, { recursive: true, force: true });
        }
        return JSON.stringify(reply);
      } catch (error) {
        if ((error as { code?: string }).code !== 'unavailable' || Date.now() >= deadline) throw error;
        after = generation;
        state.generation = 0;
      }
    }
  }
  async recoverAll(): Promise<void> {
    await Promise.allSettled([...this.owners.keys()].map(id => this.call(id, '{"op":"revision"}')));
  }
  async dispose(): Promise<void> {
    this.stopped = true;
    const owners = [...this.owners.values()]; this.owners.clear();
    await Promise.allSettled(owners.map(async pending => { const state = await pending; await state.tail; await rm(state.dir, { recursive: true, force: true }); }));
  }
}
