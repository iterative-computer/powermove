import { DOCUMENT_CHUNK_BYTES, DOCUMENT_QUEUED_WRITES, type DocumentKernelBridge, type DocumentKernelReceipt } from '../../../shared/document-kernel';
import { serializeInSlices } from '../legacy/core/json-slices';
import { canonicalChunks } from '../../../shared/canonical-async';
import { KIND, PmWasm, revive, type PmWasmExports } from './wasm';

function wireTime(t: number): number | Record<string, string> {
  if (Number.isNaN(t)) return { '\u0000': 'NaN' };
  if (t === Infinity) return { '\u0000': 'Infinity' };
  if (t === -Infinity) return { '\u0000': '-Infinity' };
  if (Object.is(t, -0)) return { '\u0000': '-0' };
  return t;
}

/** Synchronous API compatibility over the owner's native evaluator. The
 * document itself stays in the interactive utility and shares the viewer tree. */
export class RemotePmWasm extends PmWasm {
  readonly backend = 'native' as const;
  override readonly marksOwnFailures = true;
  readonly automaticRecovery: boolean;
  private divergent = false;
  private lastStatus = 0;
  private lastKind = 0;
  private receipt: DocumentKernelReceipt;

  constructor(readonly bridge: DocumentKernelBridge, receipt: DocumentKernelReceipt) {
    // Only the scalar fast path and diagnostic memory length use x outside
    // PmWasm. All public transport methods below have native implementations.
    const exports = { memory: { buffer: new ArrayBuffer(0) }, pm_ev: (id: number, t: number) => scalar(id, t), pm_kind: () => kind() } as unknown as PmWasmExports;
    super({ exports } as unknown as WebAssembly.Instance);
    this.automaticRecovery = (bridge as DocumentKernelBridge & { readonly automaticRecovery?: true }).automaticRecovery === true;
    this.receipt = receipt;
    const scalar = (id: number, t: number): number => {
      const result = this.invoke('ev', { id, t: wireTime(t) });
      this.lastKind = result.kind; this.lastStatus = result.status;
      return result.number;
    };
    const kind = () => this.lastKind;
  }

  /** A failed apply can break the renderer/native mirror even when the
   * transport itself remains healthy. Only a checked rebuild clears this. */
  get mirrorDiverged(): boolean { return this.divergent; }
  invalidateMirror(): void { this.divergent = true; this.dead = true; }

  /** Main restores this owner's revision, context, and virtual handles.
   * Keep the existing facade and all caller-owned prepared batch views. */
  resumeAutomaticRecovery(): boolean {
    if (!this.automaticRecovery || this.divergent) return false;
    this.dead = false;
    return true;
  }

  private invoke(op: string, args?: unknown, bytes?: Uint8Array): any {
    if (this.dead) throw new Error('The native document evaluator stopped');
    let reply: any;
    try { reply = JSON.parse(this.bridge.call(JSON.stringify({ op, args }), bytes)); }
    catch (error) { this.dead = true; throw error; }
    if (!reply.ok) {
      // Validation, stale preparation and bounded reader backpressure do not
      // invalidate the still-published evaluator or its interned handles.
      if (reply.code === 'unavailable') this.dead = true;
      throw Object.assign(new Error(reply.error || 'The native document evaluator failed'), { code: reply.code });
    }
    if (op === 'ev') {
      const value = reply.value;
      value.number = revive(value.number);
      if (value.lastSpecial) value.last = revive(value.last);
      return value;
    }
    return reply.special ? revive(reply.value) : reply.value;
  }

  activate(revision = this.receipt.revision): void {
    revision = Math.max(revision, this.receipt.revision);
    this.invoke('activate', { revision, preparedRevision: this.receipt.revision, hash: this.receipt.hash });
    this.receipt.revision = revision;
  }
  /** Hash the immutable native root on its bounded reader, never in a sync
   * evaluator call. A rejected/stale refresh leaves the old checkpoint valid. */
  async refresh(revision: number): Promise<DocumentKernelReceipt> {
    const deadline = Date.now() + 120_000;
    let started = false;
    for (;;) {
      if (this.revision() !== revision) throw new Error('The document changed during checkpoint refresh');
      let value: DocumentKernelReceipt | { pending: true };
      try { value = this.invoke(started ? 'pollRefresh' : 'refresh', { revision }); started = true; }
      catch (error) {
        if ((error as { code?: string }).code !== 'busy' || Date.now() >= deadline) throw error;
        await new Promise(resolve => setTimeout(resolve, 250));
        continue;
      }
      if (!value.pending) {
        if (this.revision() !== revision || value.revision !== revision) throw new Error('The document changed during checkpoint refresh');
        this.receipt = { ...value };
        return value;
      }
      if (Date.now() >= deadline) throw new Error('The document checkpoint refresh timed out');
      await new Promise(resolve => setTimeout(resolve, 5));
    }
  }
  checkpoint(receipt: DocumentKernelReceipt): void {
    this.invoke('checkpoint', { revision: receipt.revision, hash: receipt.hash });
  }
  override releaseInput(): void {}
  override releaseOutput(): void {}
  override resync(json: string, revision: number, document?: string): void { this.invoke('resync', { json, revision, document }); this.receipt = { ...this.receipt, revision, hash: '', integrity: '' }; }
  override apply(body: Uint8Array, base: number, revision: number): { code: number; message: string } {
    const result = this.invoke('apply', { base, revision }, body);
    if (result.code === 0) this.receipt = { ...this.receipt, revision, hash: '', integrity: '' };
    return result;
  }
  override revision(): number { return this.receipt.revision; }
  override hash(): string { return this.receipt.hash || (this.receipt.hash = this.invoke('hash')); }
  override consistencyHash(): string { return this.receipt.integrity || (this.receipt.integrity = this.invoke('consistencyHash')); }
  override toJson(): string { return this.invoke('toJson'); }
  override toJsonBytes(): Uint8Array { return new TextEncoder().encode(this.toJson()); }
  override intern(address: Parameters<PmWasm['intern']>[0]): number { return this.invoke('intern', address); }
  override release(id: number): void { this.invoke('release', { id }); }
  override ev(id: number, t: number): { value: unknown; threw?: string } {
    const result = this.invoke('ev', { id, t: wireTime(t) });
    this.lastKind = result.kind; this.lastStatus = result.status;
    if (result.kind === KIND.number) return { value: result.number };
    return result.kind === KIND.threw ? { value: undefined, threw: String(result.last) } : { value: result.last };
  }
  override evBatch(ids: ArrayLike<number>, t: number): { values: Float64Array; kinds: Uint8Array } {
    const result = this.invoke('evBatch', { ids: Array.from(ids), t: wireTime(t) });
    this.lastStatus = result.status;
    return { values: Float64Array.from(result.values), kinds: Uint8Array.from(result.kinds) };
  }
  override prepareBatch(ids: ArrayLike<number>): ReturnType<PmWasm['prepareBatch']> {
    const resident = Uint32Array.from(ids), values = new Float64Array(ids.length), kinds = new Uint8Array(ids.length);
    return (t, start = 0, count = resident.length - start) => {
      if (!Number.isInteger(start) || !Number.isInteger(count) || start < 0 || count < 0 || start + count > resident.length) throw new RangeError('Native batch run outside its ids');
      const answer = this.evBatch(resident.subarray(start, start + count), t);
      values.set(answer.values, start); kinds.set(answer.kinds, start);
      return { values, kinds };
    };
  }
  override evaluationStatus(): number { return this.lastStatus; }
  override expressionError(id: number): string | null { return this.invoke('expressionError', { id }); }
  override query(request: object): any { return this.invoke('query', request); }
  override define(definition: Parameters<PmWasm['define']>[0]): void { this.invoke('define', definition); this.contextRevision++; }
  override addFontAnswers(json: string): void { this.invoke('addFontAnswers', { json }); this.contextRevision++; }
  override addFont(blob: string, index: number, data: Uint8Array): void { this.invoke('addFont', { blob, index }, data); this.contextRevision++; }
  override setFontSeeds(json: string): boolean { const changed = this.invoke('setFontSeeds', { json }); if (changed) this.contextRevision++; return changed; }
  override clearFonts(): void { this.invoke('clearFonts'); this.contextRevision++; }
  override fontOutlines(enabled: boolean): void { this.invoke('fontOutlines', { enabled }); }
  override fontReport(): ReturnType<PmWasm['fontReport']> { return this.invoke('fontReport'); }
  override stats(): Record<string, number> { return this.invoke('stats'); }
  override heap(): ReturnType<PmWasm['heap']> { return this.invoke('heap'); }
}

/** Native SHA, streamed in bounded messages; no whole canonical string,
 * UTF-8 staging buffer, or JavaScript SHA state lives on the UI thread. */
export async function remoteCanonicalHash(bridge: DocumentKernelBridge, value: any, prefix = ''): Promise<string> {
  const token = await bridge.beginDigest(prefix);
  let complete = false;
  try {
    for await (const bytes of canonicalChunks(value)) await bridge.writeDigest(token, bytes);
    const hash = await bridge.endDigest(token);
    complete = true;
    return hash;
  } finally { if (!complete) await bridge.abort(token).catch(() => {}); }
}

/** The shadow's JSON text (JSON.stringify's member order, serialized in
 * slices), encoded straight into full chunks (DOCUMENT_CHUNK_BYTES) with
 * main's queue of writes in flight; a chunk's buffer is filled again once
 * main has it (one buffer per chunk of garbage otherwise). The utility parses
 * it into the same tree its canonical JSON would give and answers that tree's
 * canonical hash; main checks the file against its own hash of the stream.
 * About a tenth of the cost of the canonical stream (sorted members, a
 * generator step per token) for the large private project. */
export async function ingestDocumentText(bridge: DocumentKernelBridge, shadow: any, document: string, revision: number): Promise<DocumentKernelReceipt> {
  const token = await bridge.begin();
  let complete = false;
  try {
    const encoder = new TextEncoder();
    const inFlight: Promise<void>[] = [], free: Uint8Array<ArrayBuffer>[] = [];
    let chunk = new Uint8Array(CHUNK_BYTES), filled = 0;
    const send = async (bytes: Uint8Array<ArrayBuffer>) => {
      if (inFlight.length >= DOCUMENT_QUEUED_WRITES) await inFlight.shift();
      inFlight.push(bridge.write(token, bytes).then(() => { if (bytes.byteLength === CHUNK_BYTES) free.push(bytes); }));
    };
    // The text has no lone surrogates (JSON.stringify escapes them): it
    // encodes as it would whole, a chunk ending where the next character
    // does not fit.
    await serializeInSlices(shadow, async text => {
      for (let read = 0; read < text.length;) {
        const result = encoder.encodeInto(read ? text.slice(read) : text, chunk.subarray(filled));
        read += result.read; filled += result.written;
        if (read < text.length || filled === CHUNK_BYTES) {
          // A subarray would cross the context bridge with its whole buffer.
          const full = filled === CHUNK_BYTES;
          await send(full ? chunk : chunk.slice(0, filled));
          if (full) chunk = free.pop() ?? new Uint8Array(CHUNK_BYTES);
          filled = 0;
        }
      }
    });
    if (filled) await send(chunk.slice(0, filled));
    await Promise.all(inFlight);
    const receipt = await bridge.commit(token, document, revision);
    complete = true;
    return receipt;
  } finally { if (!complete) await bridge.abort(token).catch(() => {}); }
}
const CHUNK_BYTES = DOCUMENT_CHUNK_BYTES;

