/*
 * The pm-wasm module (engine/crates/pm-wasm): loading and the low-level calls.
 *
 * The module exports a plain C ABI over one engine per instance (see
 * engine/crates/pm-wasm/src/abi.rs). Strings and patch bodies go through a
 * grow-only input buffer, replies come back as UTF-8 JSON in a reply buffer,
 * and the batch fast path reads and writes typed-array views of linear
 * memory. Every view is re-acquired when `memory.buffer` changes (growth
 * detaches the old ArrayBuffer).
 *
 * A trap (any exception out of a wasm call) leaves the instance unusable: the
 * wrapper marks itself dead and the owner re-instantiates and resyncs.
 */

export interface PmWasmExports {
  memory: WebAssembly.Memory;
  pm_in(len: number): number;
  pm_in_release(): void;
  pm_out_ptr(): number;
  pm_out_len(): number;
  pm_out_special(): number;
  pm_resync(len: number, rev: number): number;
  pm_apply(len: number, base: number, rev: number): number;
  pm_revision(): number;
  pm_hash(): number;
  pm_consistency_hash(): number;
  pm_to_json(): number;
  pm_intern(len: number): number;
  pm_release(id: number): void;
  pm_ev(id: number, t: number): number;
  pm_kind(): number;
  pm_status_ptr(): number;
  pm_last(): number;
  pm_batch_ids(n: number): number;
  pm_batch_out(n: number): number;
  pm_batch_kinds(n: number): number;
  pm_ev_batch(n: number, t: number): number;
  pm_ev_batch_range(start: number, n: number, t: number): number;
  pm_expr_error(id: number): number;
  pm_query(len: number): number;
  pm_define(len: number): void;
  pm_fonts_answers(len: number): number;
  pm_fonts_add(len: number): number;
  pm_fonts_report(): number;
  pm_fonts_clear(): void;
  pm_fonts_seeds(len: number): number;
  pm_fonts_outlines(outlines: number): void;
  pm_heap_live(): number;
  pm_heap_peak(): number;
  pm_heap_reset_peak(): void;
  pm_stats(): number;
}

/** What `pm_apply` returns. */
export const APPLY = { ok: 0, gap: 1, revision: 2, decode: 3, mismatch: 4 } as const;

/** The value of a non-number `ev` answer. */
export const KIND = { number: 0, other: 1, threw: 2 } as const;

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/** Revives the markers pm-wasm writes for numbers JSON cannot carry. */
export function revive(value: any): any {
  if (value === null || typeof value !== 'object') return value;
  if (Array.isArray(value)) {
    for (let i = 0; i < value.length; i++) value[i] = revive(value[i]);
    return value;
  }
  const keys = Object.keys(value);
  if (keys.length === 1 && keys[0] === '\u0000') {
    switch (value['\u0000']) {
      case 'NaN': return NaN;
      case 'Infinity': return Infinity;
      case '-Infinity': return -Infinity;
      case '-0': return -0;
      case 'undefined': return undefined;
    }
  }
  for (const key of keys) value[key] = revive(value[key]);
  return value;
}

/** The module's imports: `Math.pow` from this page's own engine, so `**` in
 *  expressions equals the legacy evaluator's (docs/engine/expressions.md). */
export const imports = { pm_expr: { pow: Math.pow } };

export class PmWasm {
  /** External evaluator inputs (definition labels and font registry state). */
  contextRevision = 0;
  readonly x: PmWasmExports;
  dead = false;
  private buffer: ArrayBuffer | null = null;
  private u8: Uint8Array | null = null;
  private inPtr = 0;
  private inCap = 0;
  private statusPtr = 0;
  private statusWord: Uint32Array | null = null;
  private batchCapacity = 0;
  private batchVersion = 0;
  private batchIdsPtr = 0;
  private batchOutPtr = 0;
  private batchKindsPtr = 0;
  private activeBatch: object | null = null;

  constructor(instance: WebAssembly.Instance) {
    this.x = instance.exports as unknown as PmWasmExports;
  }

  /** Runs a call; a trap kills the instance and rethrows. */
  call<T>(fn: () => T): T {
    if (this.dead) throw new Error('pm-wasm instance is dead');
    try {
      return fn();
    } catch (error) {
      this.dead = true;
      throw error;
    }
  }

  private memory(): Uint8Array {
    const buffer = this.x.memory.buffer;
    if (buffer !== this.buffer) {
      this.buffer = buffer;
      this.u8 = new Uint8Array(buffer);
    }
    return this.u8!;
  }

  /** Writes bytes into the input buffer; returns their length. */
  writeBytes(bytes: Uint8Array): number {
    this.reserve(bytes.length);
    this.memory().set(bytes, this.inPtr);
    return bytes.length;
  }

  /** Writes a string as UTF-8 into the input buffer; returns its byte length. */
  writeString(text: string): number {
    // UTF-8 needs at most 3 bytes per UTF-16 code unit.
    let need = text.length * 3;
    if (need < 1024) need = 1024;
    this.reserve(need);
    const target = this.memory().subarray(this.inPtr, this.inPtr + this.inCap);
    const { read, written } = encoder.encodeInto(text, target);
    if (read !== text.length) throw new Error('pm-wasm: input buffer too small');
    return written;
  }

  private reserve(len: number): void {
    if (len > this.inCap || this.inPtr === 0) {
      this.inPtr = this.call(() => this.x.pm_in(len));
      this.inCap = len;
    }
  }

  /** Frees the input buffer (after a large resync). */
  releaseInput(): void {
    this.call(() => this.x.pm_in_release());
    this.inPtr = 0;
    this.inCap = 0;
  }

  /** The reply buffer as a string. */
  readOut(len = this.x.pm_out_len()): string {
    const ptr = this.x.pm_out_ptr();
    return decoder.decode(this.memory().subarray(ptr, ptr + len));
  }

  /** The reply buffer as JSON, with special numbers revived. */
  readJson(len?: number): any {
    const text = this.readOut(len);
    const value = JSON.parse(text);
    return this.x.pm_out_special() ? revive(value) : value;
  }

  /** A typed view of `n` items at `ptr` (valid until memory grows). */
  f64(ptr: number, n: number): Float64Array { this.memory(); return new Float64Array(this.buffer!, ptr, n); }
  u32(ptr: number, n: number): Uint32Array { this.memory(); return new Uint32Array(this.buffer!, ptr, n); }
  bytes(ptr: number, n: number): Uint8Array { this.memory(); return new Uint8Array(this.buffer!, ptr, n); }

  // ── the document ──────────────────────────────────────────────────────

  /** Replaces the mirror; throws with the engine's message on failure. */
  resync(json: string, revision: number): void {
    this.call(() => {
      const len = json ? this.writeString(json) : 0;
      if (this.x.pm_resync(len, revision) !== 0) throw new Error(`pm-wasm resync: ${this.readOut()}`);
    });
  }

  /** Applies a canonicalV1 body; returns the `APPLY` code (and the message). */
  apply(body: Uint8Array, base: number, revision: number): { code: number; message: string } {
    return this.call(() => {
      const len = this.writeBytes(body);
      const code = this.x.pm_apply(len, base, revision);
      return { code, message: code === APPLY.ok ? '' : this.readOut() };
    });
  }

  revision(): number { return this.call(() => this.x.pm_revision()); }
  consistencyHash(): string { return this.call(() => this.readOut(this.x.pm_consistency_hash())); }
  hash(): string { return this.call(() => this.readOut(this.x.pm_hash())); }
  toJson(): string { return this.call(() => this.readOut(this.x.pm_to_json())); }
  /** The mirror's canonical JSON as UTF-8 bytes, copied out of wasm memory:
   *  a large document never becomes a JavaScript string (C1: 170 MB). */
  toJsonBytes(): Uint8Array {
    return this.call(() => {
      const len = this.x.pm_to_json();
      const ptr = this.x.pm_out_ptr();
      return this.memory().slice(ptr, ptr + len);
    });
  }

  // ── the fast path ─────────────────────────────────────────────────────

  intern(address: { comp: string; layer: string; key: string; via: 'channel' | 'find' | Array<string | number> }): number {
    return this.call(() => this.x.pm_intern(this.writeString(JSON.stringify(address))));
  }

  release(id: number): void { this.call(() => this.x.pm_release(id)); }

  /** One id at `t`, with non-number answers and throws decoded. */
  ev(id: number, t: number): { value: unknown; threw?: string } {
    return this.call(() => {
      const n = this.x.pm_ev(id, t);
      if (n === n) return { value: n };
      const kind = this.x.pm_kind();
      if (kind === KIND.number) return { value: n };
      const value = this.readJson(this.x.pm_last());
      return kind === KIND.threw ? { value: undefined, threw: String(value) } : { value };
    });
  }

  /** Evaluates ids at `t`; returns views valid until the next call. */
  evBatch(ids: ArrayLike<number>, t: number): { values: Float64Array; kinds: Uint8Array } {
    return this.call(() => {
      const n = ids.length;
      this.reserveBatch(n);
      this.activeBatch = null;
      this.u32(this.batchIdsPtr, n).set(ids);
      if (this.x.pm_ev_batch(n, t) !== 0) throw new Error('pm-wasm: batch buffers too small');
      return { values: this.f64(this.batchOutPtr, n), kinds: this.bytes(this.batchKindsPtr, n) };
    });
  }

  /** The shared batch buffers hold at least `n` entries. Growing them moves
   * them, which `batchVersion` tells prepared batches. */
  private reserveBatch(n: number) {
    if (n <= this.batchCapacity && this.batchIdsPtr) return;
    const capacity = Math.max(n, this.batchCapacity, 1);
    this.batchIdsPtr = this.x.pm_batch_ids(capacity);
    this.batchOutPtr = this.x.pm_batch_out(capacity);
    this.batchKindsPtr = this.x.pm_batch_kinds(capacity);
    this.batchCapacity = capacity;
    this.batchVersion += 1;
    this.activeBatch = null;
  }

  /** Freeze resident ids once. Each call evaluates the run
   * `[start, start + count)` of them (all by default) into the same positions of
   * the returned views, which stay valid until the next batch call. The ids
   * are copied in again only after another caller used the shared buffers.
   * Memory growth renews views. */
  prepareBatch(ids: ArrayLike<number>): (time: number, start?: number, count?: number) => { values: Float64Array; kinds: Uint8Array } {
    const resident = Uint32Array.from(ids), n = resident.length, token = {};
    let version = -1, buffer: ArrayBuffer | null = null;
    let input: Uint32Array, values: Float64Array, kinds: Uint8Array;
    const views = () => {
      buffer = this.x.memory.buffer;
      version = this.batchVersion;
      input = new Uint32Array(buffer, this.batchIdsPtr, n);
      values = new Float64Array(buffer, this.batchOutPtr, n);
      kinds = new Uint8Array(buffer, this.batchKindsPtr, n);
    };
    return (time, start = 0, count = n - start) => {
      if (this.dead) throw new Error('pm-wasm instance is dead');
      if (!(start >= 0 && count >= 0 && start + count <= n)) throw new RangeError('pm-wasm: batch run outside its ids');
      try {
        this.reserveBatch(n);
        if (version !== this.batchVersion || buffer !== this.x.memory.buffer) views();
        // Another batch (or evBatch) wrote the shared ids since this one ran.
        if (this.activeBatch !== token) { input.set(resident); this.activeBatch = token; }
        if (this.x.pm_ev_batch_range(start, count, time) !== 0) throw new Error('pm-wasm: batch buffers too small');
        // Evaluation itself can grow memory (cold plans, errors, keyed values).
        if (buffer !== this.x.memory.buffer) views();
        return { values, kinds };
      } catch (error) {
        this.dead = true;
        throw error;
      }
    };
  }

  /** Reacquire after growth; no per-expression wasm/error-string crossing. */
  evaluationStatus(): number {
    this.statusPtr ||= this.x.pm_status_ptr();
    if (!this.statusWord || this.statusWord.buffer !== this.x.memory.buffer) this.statusWord = this.u32(this.statusPtr, 1);
    return this.statusWord[0]!;
  }

  expressionError(id: number): string | null {
    return this.call(() => {
      const len = this.x.pm_expr_error(id);
      return len < 0 ? null : this.readOut(len);
    });
  }

  // ── queries ───────────────────────────────────────────────────────────

  /** One JSON request; the parsed reply (`{ok}`, `{throw}`, `{host}`, `{fonts}`, `{error}`). */
  query(request: object): any {
    return this.call(() => this.readJson(this.x.pm_query(this.writeString(JSON.stringify(request)))));
  }

  define(definition: { id: string; label?: string; params?: Record<string, string> }): void {
    this.contextRevision += 1;
    this.call(() => this.x.pm_define(this.writeString(JSON.stringify(definition))));
  }

  addFontAnswers(json: string): void {
    this.contextRevision += 1;
    this.call(() => {
      if (this.x.pm_fonts_answers(this.writeString(json)) !== 0) throw new Error(`pm-wasm fonts: ${this.readOut()}`);
    });
  }

  addFont(blob: string, index: number, data: Uint8Array): void {
    this.contextRevision += 1;
    this.call(() => {
      const key = encoder.encode(blob);
      const bytes = new Uint8Array(8 + key.length + data.length);
      const view = new DataView(bytes.buffer);
      view.setUint32(0, key.length, true);
      bytes.set(key, 4);
      view.setUint32(4 + key.length, index, true);
      bytes.set(data, 8 + key.length);
      if (this.x.pm_fonts_add(this.writeBytes(bytes)) !== 0) throw new Error('pm-wasm: malformed font');
    });
  }

  /** The window's font cache seeds (`{ seeds }` JSON from main): text
   *  measures as this renderer's Canvas2D does. Returns whether they changed. */
  setFontSeeds(json: string): boolean {
    return this.call(() => {
      const r = this.x.pm_fonts_seeds(this.writeString(json));
      if (r === 2) throw new Error(`pm-wasm font seeds: ${this.readOut()}`);
      if (r === 1) this.contextRevision += 1;
      return r === 1;
    });
  }

  clearFonts(): void {
    this.contextRevision += 1;
    this.call(() => this.x.pm_fonts_clear());
  }

  fontOutlines(enabled: boolean): void {
    this.contextRevision += 1;
    this.call(() => this.x.pm_fonts_outlines(enabled ? 1 : 0));
  }

  fontReport(): { misses: unknown[]; pending: Array<[string, number]> } {
    return this.call(() => JSON.parse(this.readOut(this.x.pm_fonts_report())));
  }

  stats(): Record<string, number> { return this.call(() => JSON.parse(this.readOut(this.x.pm_stats()))); }

  heap(): { live: number; peak: number; memory: number } {
    return this.call(() => ({ live: this.x.pm_heap_live(), peak: this.x.pm_heap_peak(), memory: this.x.memory.buffer.byteLength }));
  }
}

/** Compiles the module once per page; instances are cheap. */
let compiled: Promise<WebAssembly.Module> | null = null;
/** Last successful load; streaming compilation includes delivery of the body. */
let compileTiming: { sourceMs: number; compileMs: number } | null = null;
export function moduleCompileTiming(): typeof compileTiming { return compileTiming; }

/** Where the editor serves the module (`public/engine/`, see engine/crates/pm-wasm/scripts/build.mjs). */
export const MODULE_PATH = 'engine/pm_wasm.wasm';

type EditorBoot = { enabled: boolean; module?: Promise<WebAssembly.Module>; timing?: { sourceMs: number; compileMs: number } };
const editorBoot: EditorBoot | undefined = (globalThis as any).__powermoveDocumentBoot;
if (editorBoot) delete (globalThis as any).__powermoveDocumentBoot;

/** The preload read the launch setting once before the renderer module graph. */
export function preloadedEngineEnabled(): boolean | undefined { return editorBoot?.enabled; }

/** Reuse compilation that overlapped module loading, or load in a standalone renderer. */
export function compileEditorModule(): Promise<WebAssembly.Module> {
  if (!compiled && editorBoot?.module) {
    compiled = editorBoot.module.then(module => { compileTiming = editorBoot.timing ?? null; return module; });
    delete editorBoot.module;
    compiled.catch(() => { compiled = null; });
  }
  return compiled ?? compileFrom(() => fetch(new URL(MODULE_PATH, globalThis.document?.baseURI ?? globalThis.location?.href).href));
}

export function compileFrom(source: () => Promise<Response | ArrayBuffer | Uint8Array>): Promise<WebAssembly.Module> {
  compiled ??= (async () => {
    const started = performance.now();
    const got = await source();
    const fetched = performance.now();
    const compile = async (): Promise<WebAssembly.Module> => {
      if (got instanceof Response) {
        if (!got.ok) throw new Error(`pm-wasm: ${got.status} fetching the module`);
        if (typeof WebAssembly.compileStreaming === 'function' && got.headers.get('content-type')?.includes('wasm')) {
          return WebAssembly.compileStreaming(got);
        }
        return WebAssembly.compile(await got.arrayBuffer());
      }
      return WebAssembly.compile(got as BufferSource);
    };
    const module = await compile();
    compileTiming = { sourceMs: fetched - started, compileMs: performance.now() - fetched };
    return module;
  })();
  compiled.catch(() => { compiled = null; });
  return compiled;
}

/** A fresh instance of a compiled module. */
export async function instantiate(module: WebAssembly.Module): Promise<PmWasm> {
  const instance = await WebAssembly.instantiate(module, imports);
  return new PmWasm(instance);
}

/** For tests: forget the compiled module. */
export function resetCompiled(): void { compiled = null; compileTiming = null; }
