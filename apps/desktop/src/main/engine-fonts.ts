/* The native font registry for pm-wasm's text layout (native engine plan,
   section 12, docs/engine/text.md).

   The renderer asks main over the `engine-fonts:*` channels: batched
   registry answers for the layout's misses, metrics-and-shaping subsets for
   the faces those answers name, the level 2 fallback manifest, and project
   fonts. Answers, manifests, and face lists cross as pm-text's JSON strings
   and go into pm-wasm untouched. Two providers serve the same channels:

   - "Use New Engine" on (Phase 2): the registry and subsetting run in the
     interactive engine (pm-node's font service). Main holds a font port to
     it (`EngineFontClient`) and relays requests and replies; it keeps only
     what a restarted engine needs (the project fonts, replayed in order) and
     never loads a registry of its own.
   - "Use New Engine" off, POWERMOVE_ENGINE_FONTS=1: main hosts the registry
     through the pm-bridge addon (`FontRegistry`), as before Phase 2.
     Development builds find the addon at engine/target/pm-bridge/
     pm_bridge.node (engine/crates/pm-bridge/scripts/build.sh);
     POWERMOVE_PM_BRIDGE overrides the path.

   With both off nothing loads and no channel is registered. With a flag on
   and no addon, main logs why and carries on without it. */
import type { IpcMain, IpcMainInvokeEvent } from 'electron';
import { existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';

import { ENGINE_FONTS_IPC, type EngineFontsResult, type FontFaceDescriptors } from '../shared/engine-fonts-ipc';
import type { PortLike } from './engine/ports';

export const ENGINE_FONTS_FLAG = 'POWERMOVE_ENGINE_FONTS';
export const ENGINE_BRIDGE_PATH = 'POWERMOVE_PM_BRIDGE';

export { ENGINE_FONTS_IPC };
export type { EngineFontsResult, FontFaceDescriptors };

/** Main's argument limits; the engine's font service checks the same family, descriptor, font, and face index limits (pm-node `fonts`). */
export const ENGINE_FONTS_LIMITS = {
  queryJsonChars: 4 * 1024 * 1024,
  familyChars: 512,
  descriptorChars: 128,
  /** Large CJK collections run to about 70 MB. */
  fontBytes: 128 * 1024 * 1024,
  faceIndex: 0xffff
} as const;

/** pm-bridge's `FontRegistry`. */
export interface NativeFontRegistry {
  readonly generation: number;
  familyNames(): string[];
  familyFaces(family: string): string;
  fallbackManifest(baseFamily: string): string;
  addProjectFont(family: string, data: Uint8Array, descriptors?: FontFaceDescriptors, index?: number): void;
  setFontSource(family: string, data: Uint8Array): void;
  clearProjectFonts(): void;
  answer(queries: string): string;
  subset(blob: string, index: number): Uint8Array;
}

type MaybePromise<T> = T | Promise<T>;

/** What serves the channels: main's own registry (values) or the engine's
 *  font service (promises). */
export interface FontsProvider {
  /** `known` is the registry generation the asking renderer last saw. */
  answer(queries: string, known: number | null): MaybePromise<string>;
  subset(blob: string, index: number): MaybePromise<Uint8Array>;
  manifest(base: string): MaybePromise<string>;
  familyFaces(family: string): MaybePromise<string>;
  addProjectFont(family: string, data: Uint8Array, descriptors: FontFaceDescriptors, index: number | undefined): MaybePromise<number>;
  setFontSource(family: string, data: Uint8Array): MaybePromise<number>;
  clearProjectFonts(): MaybePromise<number>;
  generation(): MaybePromise<number>;
}

/** Main's own registry as a provider (the flag-off path). */
export function registryProvider(registry: NativeFontRegistry): FontsProvider {
  return {
    answer: queries => registry.answer(queries),
    subset: (blob, index) => registry.subset(blob, index),
    manifest: base => registry.fallbackManifest(base),
    familyFaces: family => registry.familyFaces(family),
    addProjectFont(family, data, descriptors, index) {
      registry.addProjectFont(family, data, descriptors, index);
      return registry.generation;
    },
    setFontSource(family, data) {
      registry.setFontSource(family, data);
      return registry.generation;
    },
    clearProjectFonts() {
      registry.clearProjectFonts();
      return registry.generation;
    },
    generation: () => registry.generation
  };
}

export interface EngineFontsContext {
  /** Main's sender check: an editor window's main frame. */
  isTrustedSender(event: IpcMainInvokeEvent): boolean;
}

export function engineFontsEnabled(env: NodeJS.ProcessEnv): boolean {
  return env[ENGINE_FONTS_FLAG] === '1';
}

/** Where the addon is: the override, or the development build next to the app. */
export function bridgeAddonPath(env: NodeJS.ProcessEnv, appPath: string): string {
  return env[ENGINE_BRIDGE_PATH] || path.resolve(appPath, '../../engine/target/pm-bridge/pm_bridge.node');
}

/** Loads the addon and creates the registry (on main's thread, as CoreText matching requires). */
export function loadNativeFontRegistry(file: string, load: (file: string) => unknown = createRequire(__filename)): NativeFontRegistry {
  const addon = load(file) as { FontRegistry?: new () => NativeFontRegistry };
  if (typeof addon?.FontRegistry !== 'function') throw new Error(`${file} does not export FontRegistry`);
  return new addon.FontRegistry();
}

const BLOB_KEY = /^[0-9a-f]{32}$/;

function isText(value: unknown, max: number): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= max;
}

function isIndex(value: unknown): value is number {
  return Number.isInteger(value) && (value as number) >= 0 && (value as number) <= ENGINE_FONTS_LIMITS.faceIndex;
}

function fontBytes(value: unknown): Uint8Array | null {
  const bytes = value instanceof ArrayBuffer ? new Uint8Array(value) : ArrayBuffer.isView(value) ? new Uint8Array(value.buffer, value.byteOffset, value.byteLength) : null;
  return bytes && bytes.byteLength > 0 && bytes.byteLength <= ENGINE_FONTS_LIMITS.fontBytes ? bytes : null;
}

function descriptors(value: unknown): FontFaceDescriptors | null {
  if (value === undefined || value === null) return {};
  if (typeof value !== 'object' || Array.isArray(value)) return null;
  const out: FontFaceDescriptors = {};
  for (const key of ['weight', 'style', 'stretch'] as const) {
    const v = (value as Record<string, unknown>)[key];
    if (v === undefined) continue;
    if (typeof v !== 'string' || v.length > ENGINE_FONTS_LIMITS.descriptorChars) return null;
    out[key] = v;
  }
  return out;
}

const message = (error: unknown): string => (error instanceof Error ? error.message : String(error));

/** Runs a provider call; a value stays a value (main's registry answers
 *  synchronously), a promise stays a promise (the engine's service). */
function run<T>(fn: () => MaybePromise<T>, then?: (value: T) => void): MaybePromise<EngineFontsResult<T>> {
  const ok = (value: T): EngineFontsResult<T> => {
    then?.(value);
    return { ok: true, value };
  };
  try {
    const value = fn();
    return value instanceof Promise ? value.then(ok, (error: unknown) => ({ ok: false, error: message(error) })) : ok(value);
  } catch (error) {
    return { ok: false, error: message(error) };
  }
}

const invalid = (what: string): EngineFontsResult<never> => ({ ok: false, error: `invalid ${what}` });

/** The generation an answer reply carries, read without parsing the whole
 *  reply: pm-text's `Answers::reply_json` (used by pm-node and pm-bridge)
 *  writes `"generation":N` as the last key, and its test pins that. */
export function answerGeneration(json: string): number | null {
  const match = /"generation":(\d+)\}\s*$/.exec(json.slice(-40));
  return match ? Number(match[1]) : null;
}

type IpcLike = Pick<IpcMain, 'handle'>;

/** Which path registered the channels on an `ipcMain`: the engine's font
 *  service (installed first, at engine start) wins over main's registry. */
const owners = new WeakMap<object, 'engine' | 'main'>();

/** Registers the channels over a provider. Untrusted senders get null. */
export function registerEngineFontsIpc(ipc: IpcLike, ctx: EngineFontsContext, source: NativeFontRegistry | FontsProvider): void {
  const provider: FontsProvider = 'fallbackManifest' in source ? registryProvider(source) : source;
  /** The generation each renderer last received, for the engine's handling
   *  of answers asked under an earlier generation. */
  const seen = new Map<number, number>();
  const handle = (channel: string, fn: (sender: number, ...args: unknown[]) => MaybePromise<EngineFontsResult<unknown>>) =>
    ipc.handle(channel, (event, ...args: unknown[]) =>
      ctx.isTrustedSender(event) ? fn(event.sender?.id ?? 0, ...args) : null);

  handle(ENGINE_FONTS_IPC.answer, (sender, queries) =>
    isText(queries, ENGINE_FONTS_LIMITS.queryJsonChars)
      ? run(() => provider.answer(queries, seen.get(sender) ?? null), (json) => {
          const generation = answerGeneration(json);
          if (generation !== null) seen.set(sender, generation);
        })
      : invalid('queries'));
  handle(ENGINE_FONTS_IPC.subset, (_, blob, index) => (typeof blob === 'string' && BLOB_KEY.test(blob) && isIndex(index) ? run(() => provider.subset(blob, index)) : invalid('subset key')));
  handle(ENGINE_FONTS_IPC.manifest, (_, base) => (isText(base, ENGINE_FONTS_LIMITS.familyChars) ? run(() => provider.manifest(base)) : invalid('family')));
  handle(ENGINE_FONTS_IPC.familyFaces, (_, family) => (isText(family, ENGINE_FONTS_LIMITS.familyChars) ? run(() => provider.familyFaces(family)) : invalid('family')));
  handle(ENGINE_FONTS_IPC.addProjectFont, (_, family, data, desc, index) => {
    const bytes = fontBytes(data);
    const d = descriptors(desc);
    if (!isText(family, ENGINE_FONTS_LIMITS.familyChars) || !bytes || !d || (index !== undefined && !isIndex(index))) return invalid('project font');
    return run(() => provider.addProjectFont(family, bytes, d, index as number | undefined));
  });
  handle(ENGINE_FONTS_IPC.setFontSource, (_, family, data) => {
    const bytes = fontBytes(data);
    if (!isText(family, ENGINE_FONTS_LIMITS.familyChars) || !bytes) return invalid('font source');
    return run(() => provider.setFontSource(family, bytes));
  });
  handle(ENGINE_FONTS_IPC.clearProjectFonts, () => run(() => provider.clearProjectFonts()));
  handle(ENGINE_FONTS_IPC.generation, () => run(() => provider.generation()));
}

export interface EngineFontsSetup {
  ipc: IpcLike;
  ctx: EngineFontsContext;
  env: NodeJS.ProcessEnv;
  appPath: string;
  log?: (message: string) => void;
  load?: (file: string) => unknown;
}

/** Main's own registry, behind POWERMOVE_ENGINE_FONTS=1 with "Use New
 *  Engine" off. Returns the registry, or null when off, unavailable, or when
 *  the engine's font service already serves the channels. */
export function setupEngineFonts({ ipc, ctx, env, appPath, log = console.warn, load }: EngineFontsSetup): NativeFontRegistry | null {
  if (owners.get(ipc) === 'engine') return null;
  if (!engineFontsEnabled(env)) return null;
  const file = bridgeAddonPath(env, appPath);
  if (!existsSync(file)) {
    log(`[engine-fonts] ${ENGINE_FONTS_FLAG}=1 but the pm-bridge addon is missing at ${file} (engine/crates/pm-bridge/scripts/build.sh builds it)`);
    return null;
  }
  try {
    const registry = loadNativeFontRegistry(file, load);
    registerEngineFontsIpc(ipc, ctx, registry);
    owners.set(ipc, 'main');
    return registry;
  } catch (error) {
    log(`[engine-fonts] could not load the font registry from ${file}: ${message(error)}`);
    return null;
  }
}

/* ── The engine's font service (Use New Engine) ──────────────────────── */

/** Operations of the font port's wire format (pm-node `fonts::wire::Op`). */
export const FONT_OP = {
  open: 1,
  answer: 2,
  subset: 3,
  manifest: 4,
  familyFaces: 5,
  addProjectFont: 6,
  setFontSource: 7,
  clearProjectFonts: 8,
  generation: 9,
  familyNames: 10
} as const;

/** Reply statuses (pm-node `fonts::wire::Status`). */
export const FONT_STATUS = { json: 0, bytes: 1, error: 2 } as const;

type FontOp = (typeof FONT_OP)[keyof typeof FONT_OP];

const utf8 = new TextEncoder();
const fromUtf8 = new TextDecoder('utf-8', { fatal: true });

/** `id u32 LE | op u8 | json length u32 LE | JSON | binary tail`. */
export function encodeFontRequest(id: number, op: FontOp, json: string, tail?: Uint8Array): Uint8Array {
  const text = utf8.encode(json);
  const out = new Uint8Array(9 + text.byteLength + (tail?.byteLength ?? 0));
  const view = new DataView(out.buffer);
  view.setUint32(0, id, true);
  view.setUint8(4, op);
  view.setUint32(5, text.byteLength, true);
  out.set(text, 9);
  if (tail) out.set(tail, 9 + text.byteLength);
  return out;
}

/** `id u32 LE | status u8 | payload`, or null when it is not a reply. */
export function decodeFontReply(data: unknown): { id: number; status: number; payload: Uint8Array } | null {
  if (!(data instanceof Uint8Array) || data.byteLength < 5) return null;
  const status = data[4]!;
  if (status !== FONT_STATUS.json && status !== FONT_STATUS.bytes && status !== FONT_STATUS.error) return null;
  return { id: new DataView(data.buffer, data.byteOffset, 4).getUint32(0, true), status, payload: data.subarray(5) };
}

/** Changes a restarted engine must see again, in order. */
type ProjectChange = { op: typeof FONT_OP.addProjectFont | typeof FONT_OP.setFontSource; json: string; bytes: Uint8Array };

type Pending = {
  op: FontOp;
  json: string;
  tail: Uint8Array | undefined;
  /** The open and the replay: sent again by every new port, never requeued. */
  internal: boolean;
  /** The project font change this request makes; it joins the replay when
   *  the request goes out on a port (`send`), not before. */
  change: ProjectChange | undefined;
  /** The caller already got its answer (it timed out after the request went
   *  out): a late reply only keeps the replay in step with the engine. */
  settled: boolean;
  resolve: (reply: { status: number; payload: Uint8Array }) => void;
  reject: (error: Error) => void;
  timer: { cancel(): void } | null;
};

export type EngineFontClientOptions = {
  log?: (level: 'info' | 'warn' | 'error', message: string) => void;
  /** How long a request may wait for the engine (queued while it restarts). */
  timeoutMs?: number;
  schedule?: (fn: () => void, ms: number) => { cancel(): void };
};

const MUTATING = new Set<FontOp>([FONT_OP.addProjectFont, FONT_OP.setFontSource, FONT_OP.clearProjectFonts]);

/**
 * Main's end of the font port to the interactive engine's font service.
 *
 * Requests made while the engine is down (starting, or restarting after a
 * crash) wait for the next port. Every new port opens with a generation
 * base above every generation main has seen, then gets the project fonts
 * replayed, so a restarted engine never reuses an earlier launch's
 * generations: the renderer sees a new generation and starts its answers
 * again. Then the waiting requests go out in order.
 *
 * The replay holds exactly the project font changes that went out on a
 * port: a change joins it when it is sent, and a clear empties it when it
 * is sent, so a change made while the engine is down reaches the engine
 * once (with the waiting requests, after the replay). Read-only requests in
 * flight when a port goes away are sent again on the next one; project font
 * changes in flight are already in the replay and settle with the replay's
 * generation. A change leaves the replay only when the engine refuses it,
 * even after its caller timed out, since the engine may still apply a
 * request that went out.
 */
export class EngineFontClient implements FontsProvider {
  private port: PortLike | null = null;
  private nextId = 1;
  private pending = new Map<number, Pending>();
  /** Requests waiting for a port, in order. */
  private queue: Pending[] = [];
  private project: ProjectChange[] = [];
  private maxGeneration = 0;
  private readonly log: NonNullable<EngineFontClientOptions['log']>;
  private readonly timeoutMs: number;
  private readonly schedule: NonNullable<EngineFontClientOptions['schedule']>;

  constructor(options: EngineFontClientOptions = {}) {
    this.log = options.log ?? (() => {});
    this.timeoutMs = options.timeoutMs ?? 30_000;
    this.schedule = options.schedule ?? ((fn, ms) => {
      const t = setTimeout(fn, ms);
      return { cancel: () => clearTimeout(t) };
    });
  }

  /** Whether a font port to a running engine is connected. */
  get connected(): boolean {
    return this.port !== null;
  }

  /** The highest registry generation any reply carried. */
  get generationSeen(): number {
    return this.maxGeneration;
  }

  /** A new font port (an engine launch). Replaces the current one. */
  connect(port: PortLike): void {
    const previous = this.port;
    this.port = port;
    if (previous) previous.close();
    this.requeueInFlight();
    port.on('message', (event) => this.onReply(port, event.data));
    port.on('close', () => {
      if (this.port !== port) return;
      this.port = null;
      this.requeueInFlight();
    });
    port.start();
    // The open and the replay go first; queued requests follow in order.
    const base = this.maxGeneration + 1;
    this.send(this.internal(FONT_OP.open, JSON.stringify({ generationBase: base }), undefined));
    for (const change of this.project) this.send(this.internal(change.op, change.json, change.bytes));
    const queued = this.queue;
    this.queue = [];
    for (const p of queued) this.send(p);
  }

  /** The port went away for good (the engine stopped). Waiting requests fail. */
  disconnect(reason = 'the interactive engine stopped'): void {
    this.port?.close();
    this.port = null;
    const all = [...this.pending.values(), ...this.queue];
    this.pending.clear();
    this.queue = [];
    for (const p of all) {
      p.timer?.cancel();
      p.reject(new Error(reason));
    }
  }

  answer(queries: string, known: number | null): Promise<string> {
    // `queries` is the renderer's JSON array; splice it in without parsing.
    const json = `{"queries":${queries}${known === null ? '' : `,"known":${known}`}}`;
    return this.call(FONT_OP.answer, json).then((r) => this.json(r));
  }

  subset(blob: string, index: number): Promise<Uint8Array> {
    return this.call(FONT_OP.subset, JSON.stringify({ blob, index })).then((r) => {
      if (r.status !== FONT_STATUS.bytes) throw new Error('the font service sent no subset bytes');
      return r.payload;
    });
  }

  manifest(base: string): Promise<string> {
    return this.call(FONT_OP.manifest, JSON.stringify({ base })).then((r) => this.json(r));
  }

  familyFaces(family: string): Promise<string> {
    return this.call(FONT_OP.familyFaces, JSON.stringify({ family })).then((r) => this.json(r));
  }

  familyNames(): Promise<string[]> {
    return this.call(FONT_OP.familyNames, '{}').then((r) => JSON.parse(this.json(r)) as string[]);
  }

  addProjectFont(family: string, data: Uint8Array, descriptors: FontFaceDescriptors, index: number | undefined): Promise<number> {
    const json = JSON.stringify({ family, ...descriptors, ...(index === undefined ? {} : { index }) });
    return this.change({ op: FONT_OP.addProjectFont, json, bytes: data.slice() });
  }

  setFontSource(family: string, data: Uint8Array): Promise<number> {
    return this.change({ op: FONT_OP.setFontSource, json: JSON.stringify({ family }), bytes: data.slice() });
  }

  clearProjectFonts(): Promise<number> {
    return this.call(FONT_OP.clearProjectFonts, '{}').then((r) => this.generationOf(r));
  }

  generation(): Promise<number> {
    return this.call(FONT_OP.generation, '{}').then((r) => this.generationOf(r));
  }

  /** A project font change. It joins the replay when sent (so a restart
   *  while it is in flight replays it) and leaves it if the engine refuses
   *  it (data that is not a font, invalid descriptors). */
  private change(change: ProjectChange): Promise<number> {
    return this.call(change.op, change.json, change.bytes, change).then((r) => this.generationOf(r));
  }

  private json(reply: { status: number; payload: Uint8Array }): string {
    if (reply.status !== FONT_STATUS.json) throw new Error('the font service sent no JSON');
    const text = fromUtf8.decode(reply.payload);
    const generation = answerGeneration(text);
    if (generation !== null) this.maxGeneration = Math.max(this.maxGeneration, generation);
    return text;
  }

  private generationOf(reply: { status: number; payload: Uint8Array }): number {
    const value = (JSON.parse(this.json(reply)) as { generation?: unknown }).generation;
    if (typeof value !== 'number') throw new Error('the font service sent no generation');
    this.maxGeneration = Math.max(this.maxGeneration, value);
    return value;
  }

  /** The open and replayed changes: their replies only move the generation. */
  private internal(op: FontOp, json: string, tail: Uint8Array | undefined): Pending {
    return {
      op,
      json,
      tail,
      internal: true,
      resolve: (reply) => {
        if (reply.status === FONT_STATUS.error) {
          this.log('warn', `[engine-fonts] the engine refused a replayed project font: ${fromUtf8.decode(reply.payload)}`);
          return;
        }
        try {
          if (reply.status === FONT_STATUS.json) this.generationOf(reply);
        } catch (error) {
          this.log('warn', `[engine-fonts] ${message(error)}`);
        }
      },
      reject: () => {},
      change: undefined,
      settled: false,
      timer: null
    };
  }

  private call(op: FontOp, json: string, tail?: Uint8Array, change?: ProjectChange): Promise<{ status: number; payload: Uint8Array }> {
    return new Promise((resolve, reject) => {
      const p: Pending = {
        op,
        json,
        tail,
        internal: false,
        resolve: (reply) => {
          if (reply.status === FONT_STATUS.error) {
            // The engine did not apply it: a restart must not replay it.
            if (p.change) this.project = this.project.filter((c) => c !== p.change);
            if (!p.settled) reject(new Error(fromUtf8.decode(reply.payload)));
          } else if (p.settled) {
            if (reply.status === FONT_STATUS.json) this.noteGeneration(reply);
          } else resolve(reply);
        },
        reject,
        change,
        settled: false,
        timer: null
      };
      p.timer = this.schedule(() => {
        p.settled = true;
        // Not sent yet: it never will be. Sent: it stays in flight, so a
        // late reply still tells whether the engine applied it.
        this.queue = this.queue.filter((q) => q !== p);
        reject(new Error(`the font service did not answer within ${this.timeoutMs} ms`));
      }, this.timeoutMs);
      if (this.port) this.send(p);
      else this.queue.push(p);
    });
  }

  /** The generation a reply carries, for replies nobody waits for. */
  private noteGeneration(reply: { status: number; payload: Uint8Array }): void {
    try {
      this.generationOf(reply);
    } catch {
      // Not a generation reply (a read-only request that timed out).
    }
  }

  private send(p: Pending): void {
    if (!p.internal) {
      // The replay follows what went out, in order.
      if (p.change) this.project.push(p.change);
      else if (p.op === FONT_OP.clearProjectFonts) this.project = [];
    }
    const id = this.nextId++;
    if (this.nextId > 0xffff_ffff) this.nextId = 1;
    this.pending.set(id, p);
    this.port!.postMessage(encodeFontRequest(id, p.op, p.json, p.tail));
  }

  private onReply(port: PortLike, data: unknown): void {
    if (port !== this.port) return;
    const reply = decodeFontReply(data);
    if (!reply) {
      this.log('warn', '[engine-fonts] a malformed message on the font port was dropped');
      return;
    }
    const p = this.pending.get(reply.id);
    if (!p) return;
    this.pending.delete(reply.id);
    p.timer?.cancel();
    p.resolve(reply);
  }

  /** In-flight requests of a port that went away: read-only ones go back in
   *  the queue; project font changes are in the replay already, so they
   *  settle with the generation after the next port's replay. The open and
   *  the replay itself are sent again by the next port. */
  private requeueInFlight(): void {
    const inFlight = [...this.pending.values()];
    this.pending.clear();
    // Settled requests have no caller left; their changes stay in the replay.
    const again = inFlight.filter((p) => !p.internal && !p.settled);
    for (const p of again) {
      if (!MUTATING.has(p.op)) continue;
      p.op = FONT_OP.generation;
      p.json = '{}';
      p.tail = undefined;
      p.change = undefined;
    }
    this.queue = [...again, ...this.queue];
  }
}

/** Installs the engine's font service as the channels' provider (Use New
 *  Engine on). Main's own registry is then never loaded. Returns the
 *  uninstall (handlers removed). */
export function installEngineFontService(ipc: IpcLike & Pick<IpcMain, 'removeHandler'>, ctx: EngineFontsContext, client: EngineFontClient): () => void {
  if (owners.has(ipc)) throw new Error('the engine-fonts channels are already registered');
  registerEngineFontsIpc(ipc, ctx, client);
  owners.set(ipc, 'engine');
  return () => {
    for (const channel of Object.values(ENGINE_FONTS_IPC)) ipc.removeHandler(channel);
    owners.delete(ipc);
  };
}
