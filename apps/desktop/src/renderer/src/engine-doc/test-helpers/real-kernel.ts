import { createHash, type Hash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { canonicalStringify } from '../../../../shared/canonical-json';
import type { DocumentKernelBridge, DocumentKernelReceipt } from '../../../../shared/document-kernel';
import { digestInstructions, type DigestInstruction } from '../../../../shared/document-digest';
import { imports, KIND, PmWasm, revive } from '../wasm';

const module = WebAssembly.compile(readFileSync(fileURLToPath(new URL('../../../public/engine/pm_wasm.wasm', import.meta.url))));
export const loadTestModule = () => module;
export const realWasm = async () => new PmWasm(await WebAssembly.instantiate(await module, imports));

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>(done => { resolve = done; });
  return { promise, resolve };
}

export function numberMarker(n: number): number | Record<string, string> {
  if (Number.isNaN(n)) return { '\u0000': 'NaN' };
  if (n === Infinity) return { '\u0000': 'Infinity' };
  if (n === -Infinity) return { '\u0000': '-Infinity' };
  if (Object.is(n, -0)) return { '\u0000': '-0' };
  return n;
}

/** A protocol fixture, not an evaluator mock: every document, scalar, batch,
 * query, font and patch answer comes from the shipped WASM engine. The real
 * native worker/Arc-sharing path has separate Rust production-type tests. */
export class RealKernelBridge implements DocumentKernelBridge {
  readonly calls: Array<{ op: string; args: any }> = [];
  readonly streams = new Map<string, Uint8Array[]>();
  private digests = new Map<string, Hash>();
  private next = 0;
  private opened = false;
  private pending: { wasm: PmWasm; bytes: Uint8Array; document: string; receipt: DocumentKernelReceipt } | null = null;
  private commitGate: { entered: ReturnType<typeof deferred>; release: ReturnType<typeof deferred> } | null = null;
  corruptIntegrity = false;
  /** The utility's canonical hash of the parsed tree disagrees with main's hash of the stream. */
  corruptHash = false;
  currentDocument: string | null = null;

  private constructor(readonly live: PmWasm) {}
  static async create() { return new RealKernelBridge(await realWasm()); }
  get hasPrepared(): boolean { return !!this.pending; }

  holdCommit() {
    const gate = { entered: deferred(), release: deferred() };
    this.commitGate = gate;
    return { entered: gate.entered.promise, release: gate.release.resolve };
  }

  async begin(): Promise<string> { const token = `stream-${++this.next}`; this.streams.set(token, []); return token; }
  async write(token: string, chunk: Uint8Array): Promise<void> {
    const stream = this.streams.get(token);
    if (!stream) throw new Error('The document stream is closed');
    stream.push(chunk.slice());
  }
  async commit(token: string, document: string, revision: number): Promise<DocumentKernelReceipt> {
    const stream = this.streams.get(token);
    if (!stream) throw new Error('The document stream is closed');
    this.streams.delete(token);
    const bytes = Uint8Array.from(Buffer.concat(stream));
    const wasm = await realWasm();
    wasm.resync(new TextDecoder().decode(bytes), revision); wasm.releaseInput();
    const sourceHash = createHash('sha256').update(bytes).digest('hex');
    if (wasm.hash() !== sourceHash || this.corruptHash) throw new Error('The native document does not match its canonical hash');
    const receipt: DocumentKernelReceipt = { pending: false, revision, hash: sourceHash, integrity: wasm.consistencyHash() };
    this.pending = { wasm, bytes, document, receipt };
    const gate = this.commitGate; this.commitGate = null;
    if (gate) { gate.entered.resolve(); await gate.release.promise; }
    return this.corruptIntegrity ? { ...receipt, integrity: '0'.repeat(64) } : { ...receipt };
  }
  async abort(token: string): Promise<void> { this.streams.delete(token); this.digests.delete(token); }
  async beginDigest(prefix: string): Promise<string> {
    const token = `digest-${++this.next}`;
    this.digests.set(token, createHash('sha256').update(prefix)); return token;
  }
  async writeDigest(token: string, chunk: Uint8Array): Promise<void> {
    const digest = this.digests.get(token); if (!digest) throw new Error('The digest is closed'); digest.update(chunk);
  }
  async endDigest(token: string): Promise<string> {
    const digest = this.digests.get(token); if (!digest) throw new Error('The digest is closed');
    this.digests.delete(token); return digest.digest('hex');
  }
  async digestBatch(instructions: DigestInstruction[]): Promise<string[]> {
    return digestInstructions(instructions, (value, prefix = '') => createHash('sha256').update(prefix).update(canonicalStringify(value)!).digest('hex'));
  }

  call = (request: string, data?: Uint8Array): string => {
    const { op, args = {} } = JSON.parse(request);
    this.calls.push({ op, args });
    try {
      if (!this.opened && !['activate', 'poll', 'cancel', 'close', 'resync'].includes(op)) throw new Error('The document kernel has no live document');
      let value: any = null, special = false;
      switch (op) {
        case 'activate': {
          const prepared = this.pending;
          if (!prepared || prepared.receipt.hash !== args.hash || prepared.receipt.revision !== args.preparedRevision) throw new Error('The prepared replacement changed');
          if (args.revision < args.preparedRevision || (this.opened && args.revision <= this.live.revision())) throw new Error('Activation must follow the live revision');
          // Reuse the original evaluator so existing interned ids stay valid,
          // exactly as native adoption resets plans but keeps interned slots.
          this.live.resync(new TextDecoder().decode(prepared.bytes), args.revision); this.live.releaseInput();
          this.currentDocument = prepared.document; this.opened = true; this.pending = null;
          break;
        }
        case 'poll':
          if (!this.pending && !this.opened) throw new Error('The document kernel has no live document');
          value = this.pending?.receipt ?? { pending: false, revision: this.live.revision(), hash: this.live.hash(), integrity: this.live.consistencyHash() }; break;
        case 'cancel': this.pending = null; break;
        case 'close': this.pending = null; this.opened = false; break;
        case 'resync': this.live.resync(args.json, args.revision); this.live.releaseInput(); this.opened = true; this.pending = null; this.currentDocument = args.document ?? this.currentDocument; break;
        case 'revision': value = this.live.revision(); break;
        case 'refresh':
        case 'pollRefresh':
          if (args.revision !== this.live.revision()) throw new Error('The document changed during checkpoint refresh');
          value = { pending: false, revision: args.revision, hash: this.live.hash(), integrity: this.live.consistencyHash() }; break;
        case 'checkpoint':
          if (args.revision !== this.live.revision() || args.hash !== this.live.hash()) throw new Error('The checkpoint changed');
          break;
        case 'hash': value = this.live.hash(); break;
        case 'consistencyHash': value = this.live.consistencyHash(); break;
        case 'toJson': value = this.live.toJson(); break;
        case 'apply': value = this.live.apply(data ?? Uint8Array.from(args.body), args.base, args.revision); break;
        case 'intern': value = this.live.intern(args); break;
        case 'release': this.live.release(args.id); break;
        case 'expressionError': value = this.live.expressionError(args.id); break;
        case 'ev': {
          const n = this.live.x.pm_ev(args.id, revive(args.t));
          const kind = this.live.x.pm_kind(), status = this.live.evaluationStatus();
          let last: any = null, lastSpecial = false;
          if (kind !== KIND.number) { last = JSON.parse(this.live.readOut(this.live.x.pm_last())); lastSpecial = !!this.live.x.pm_out_special(); }
          value = { number: numberMarker(n), kind, last, status, lastSpecial };
          special = typeof value.number === 'object' || lastSpecial;
          break;
        }
        case 'evBatch': {
          const result = this.live.evBatch(args.ids, revive(args.t));
          const values = Array.from(result.values, numberMarker);
          value = { values, kinds: Array.from(result.kinds), status: this.live.evaluationStatus() };
          special = values.some(n => typeof n === 'object'); break;
        }
        case 'query': {
          const len = this.live.x.pm_query(this.live.writeString(JSON.stringify(args)));
          value = JSON.parse(this.live.readOut(len)); special = !!this.live.x.pm_out_special(); break;
        }
        case 'define': this.live.define(args); break;
        case 'addFontAnswers': this.live.addFontAnswers(args.json); break;
        case 'addFont': this.live.addFont(args.blob, args.index, data ?? Uint8Array.from(args.data)); break;
        case 'setFontSeeds': value = this.live.setFontSeeds(args.json); break;
        case 'clearFonts': this.live.clearFonts(); break;
        case 'fontOutlines': this.live.fontOutlines(args.enabled); break;
        case 'fontReport': value = this.live.fontReport(); break;
        case 'stats': value = this.live.stats(); break;
        case 'heap': value = this.live.heap(); break;
        default: throw new Error(`Unsupported fixture operation ${op}`);
      }
      return JSON.stringify({ ok: true, value, ...(special ? { special: true } : {}) });
    } catch (error) { return JSON.stringify({ ok: false, error: (error as Error).message }); }
  };
}
