import { effectParameterDefault } from '../src/kernel/effect-defaults';
import { resolveEffectDefinition } from '../src/kernel/effect-versions';
import { createSubscriber } from 'svelte/reactivity';
import type { PowermoveAPI, Selection } from '../src/kernel/api';
import type { PMRegistry } from '../src/legacy/registry';
import { createRpc, serializeRpcError, type Rpc, type HandleId } from '../../shared/sandbox-rpc';
import { install as installEase } from '../src/legacy/core/easing';
import { CHANNELS_3D, projectPoint, inversePlane } from '../src/legacy/core/space-3d';

export class PermissionError extends Error {
  readonly code = 'full-access';
  constructor(member: string, alternative = 'project.apply or commands') {
    super(`${member} requires full access. Use ${alternative} in a sandboxed extension.`);
    this.name = 'PermissionError';
  }
}
export class ProjectWritePermissionError extends Error {
  readonly code = 'project:write';
  constructor(member: string) {
    super(`${member} requires project:write permission. Without it, commands.run may call only commands this extension registered.`);
    this.name = 'PermissionError';
  }
}
export interface SandboxControl {
  /** The kernel's one notification per flush: apply `delta`, then dispatch `events` to local listeners in order. */
  tick(delta: Partial<SandboxState>, events: SandboxEvent[]): void;
  /** The kernel's theme push, for `theme.active()` and `theme.scheme()`. */
  theme(theme: SandboxInit['theme']): void;
  /** The kernel's catalog push, after other extensions loaded or unloaded. */
  catalog(catalog: SandboxInit['catalog']): void;
  ready(): Promise<unknown>;
  dispose(): void;
  setQuiet(on: boolean): void;
  panel(id: string): Record<string, any> | undefined;
  mountPanel(panelId: string, token: string, port: MessagePort): void;
  unmountPanel(token: string): void;
}
export const sandboxControl = (api: PowermoveAPI): SandboxControl => (api as unknown as { __sandbox: SandboxControl }).__sandbox;
/**
 * What every sandbox document holds without asking: small, pushed as deltas
 * in `tick`. `generation` names the project snapshot `project.get()` would
 * return.
 */
export interface SandboxState { time: number; playing: boolean; revision: number; generation: number; selection: Selection | null }
export type SandboxEvent = [name: string, payload: unknown];
/** Reply to `project-snapshot`. `unchanged`: this document was already sent that generation, so its copy stands. */
export type SandboxSnapshot = { generation: number; json: string } | { generation: number; tooLarge: true } | { generation: number; unchanged: true };
export interface SandboxInit {
  id: string; apiVersion: number; manifest: PowermoveAPI['manifest']; vars: Record<string, string>;
  /** `id` is the active theme; pushes carry it too. */
  theme: { id?: string; scheme: string; tokens: Record<string, string> }; state: SandboxState; bundleUrl: string;
  catalog?: Record<string, Array<Record<string, any>>>;
  activeTheme?: string;
}
/** A host keybinding as a view needs it to decide which keydowns to forward. */
export interface SandboxKey { chord: string; inFields: boolean; repeat: boolean; looseModifiers: boolean }
/** What a view forwards for a keydown the host has a binding for (or Escape). */
export interface SandboxKeyEvent { key: string; code: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; repeat: boolean; field: boolean }
export interface SandboxViewInit extends SandboxInit {
  mode: 'view'; panelId: string; spec: Record<string, unknown>; keys: SandboxKey[];
  size: { width: number; height: number }; noscroll?: boolean;
}
/*
 * What the sandbox check (kernel/sandbox-check.ts) learns from inside the
 * document: a trusted-only member was reached, or apiVersion ≤ 2 code read a
 * property of a result that is a Promise here but was a plain value in-realm.
 * Sent as `sandbox-report` notifications; capped per member so a status
 * callback that misbehaves every tick cannot flood the port.
 */
export type SandboxReportKind = 'permission' | 'async';
export type SandboxReporter = (kind: SandboxReportKind, member: string) => void;
const REPORT_CAP = 50;
export function sandboxReporter(rpc: Pick<Rpc, 'notify'>): SandboxReporter {
  const counts = new Map<string, number>();
  return (kind, member) => {
    const key = `${kind}:${member}`;
    const count = (counts.get(key) ?? 0) + 1;
    counts.set(key, count);
    if (count > REPORT_CAP) return;
    try { rpc.notify('sandbox-report', { kind, member }); } catch { /* port closed */ }
  };
}
const trustedOnly = (name: string, report: SandboxReporter, alternative?: string): any => new Proxy({}, {
  has(_target, member) {
    if (typeof member === 'symbol') return false;
    report('permission', `${name}.${member}`);
    return false;
  },
  get(_target, member) {
    if (member === 'then' || typeof member === 'symbol') return undefined;
    report('permission', `${name}.${member}`);
    throw new PermissionError(`${name}.${member}`, alternative);
  }
});
/** A sandbox-safe namespace whose other members are trusted-only (`api.ui`, `api.media`). */
const partlyTrusted = <T extends object>(name: string, safe: T, report: SandboxReporter): T => new Proxy(safe, {
  has(target, member) {
    if (member in target) return true;
    if (typeof member !== 'symbol') report('permission', `${name}.${member}`);
    return false;
  },
  get(target, member, receiver) {
    if (typeof member === 'symbol' || member === 'then' || member === 'toJSON' || member in target) return Reflect.get(target, member, receiver);
    report('permission', `${name}.${member}`);
    throw new PermissionError(`${name}.${member}`);
  }
});
/* Promise members any caller may touch; reading anything else off a pending
   result (`.ok`, `.length`, iterating, string coercion) is sync-era code. */
const PROMISE_MEMBERS = new Set<PropertyKey>(['then', 'catch', 'finally', 'constructor', Symbol.toStringTag]);
/** Wrap a Promise so a synchronous read of its "value" is reported, then behaves exactly like the Promise. */
export function watchPromise<T>(promise: Promise<T>, member: string, report: SandboxReporter): Promise<T> {
  let reported = false;
  return new Proxy(promise, {
    get(target, key) {
      if (!PROMISE_MEMBERS.has(key) && !reported) { reported = true; report('async', member); }
      const value = Reflect.get(target, key, target);
      return typeof value === 'function' ? value.bind(target) : value;
    }
  });
}
/** Panel layout hints that cross to the kernel; the definition itself never does. */
export interface SandboxPanelInfo { id: string; title: string; icon?: string; size?: number; min?: number; flush?: boolean; noscroll?: boolean }
export function panelInfo(def: Record<string, any>): SandboxPanelInfo {
  const info: SandboxPanelInfo = { id: String(def.id), title: typeof def.title === 'string' && def.title ? def.title : String(def.id) };
  if (typeof def.icon === 'string') info.icon = def.icon;
  for (const key of ['size', 'min'] as const) if (typeof def[key] === 'number' && Number.isFinite(def[key])) info[key] = def[key];
  for (const key of ['flush', 'noscroll'] as const) if (def[key] === true) info[key] = true;
  return info;
}
/*
 * Two modes, one bundle.
 *
 * `runtime` is the extension's single long-lived iframe: `activate(api)` runs
 * here and every registration is forwarded to the kernel, which owns the
 * registries.
 *
 * `view` is a panel's own iframe. It imports the SAME bundle and runs
 * `activate(api)` again only so the panel definition (a Svelte component
 * cannot cross a MessagePort) is recorded locally and can be mounted. So there
 * are two or more live instances of the extension module: the runtime plus one
 * per open panel, exactly like a VS Code webview beside its extension host.
 * Module-level variables are NOT shared between them; shared state goes
 * through `api.storage` and `api.events`. In view mode:
 *   - registries (commands, effects, panels, status, keybindings, ...) are
 *     recorded locally and never forwarded: the runtime already owns them;
 *   - event interest is forwarded, so panel UI stays live;
 *   - reads (project state and snapshots, vars, storage.get,
 *     extensions.list, ...) work;
 *   - while the view's `activate` runs, side-effecting calls (panels.open,
 *     toasts, storage.set, project edits, events.emit, ...) are dropped,
 *     because the runtime's own `activate` already performed them once.
 *
 * Data plane, the same in both modes (design: docs/sandbox-data-plane.md §3).
 * The document holds a small SandboxState (time, playing, revision,
 * generation, selection) that the kernel keeps current with at most one
 * `tick(delta, events)` per flush, so `time()`, `playing()`, `revision()` and
 * `selection()` are synchronous. The project itself is never pushed:
 * `project.get()` is async and pulls `project-snapshot` only when the cached
 * copy's generation is behind the state's, with one call in flight shared by
 * every caller; the copy is parsed and deep-frozen. `events.on` listeners stay
 * in this document: the first listener for a name registers interest with
 * the kernel and the last one withdraws it, so no callback handle crosses and
 * an occurrence costs no round trip. Reading the project needs no
 * permission; `network` is what gates sending it anywhere.
 *
 * Reactive reads (time, playing, revision, selection, latest, theme) cost
 * nothing new on the port: they re-run their readers from the ticks and theme
 * pushes that arrive anyway, and only `latest()` pulls, once per generation and
 * at most once a frame, while it has a reader.
 */
export type SandboxMode = 'runtime' | 'view';
const VIEW_READS = new Set(['storage.get', 'assets.get', 'assets.readText', 'media.getImportDefaults', 'ui.icon', 'panels.isOpen']);
/*
 * A reactive read calls `read()`: inside a template, $derived or $effect that
 * subscribes the reader (the first one runs `start`), anywhere else it does
 * nothing. `bump()` re-runs the current readers; with none it is a null check.
 */
function reactiveSource(start?: () => void): { read(): void; bump(): void; live(): boolean } {
  let update: (() => void) | null = null;
  const read = createSubscriber(next => { update = next; start?.(); return () => { update = null; }; });
  return { read, bump: () => update?.(), live: () => update !== null };
}
/*
 * Callback handles in the replies to a callback the host asks again and
 * again (palette queries, menu opens). Replies can land in any order and the
 * host keeps the one for the newest call it made, so handles are kept by the
 * order the calls were made, not the order they answered: the two newest
 * replies stay runnable, and a reply two newer ones already beat creates none.
 */
function replyHandles(handle: (fn: (...args: any[]) => unknown) => HandleId, release: (id: HandleId) => void) {
  let issued = 0, disposed = false;
  let kept: Array<{ seq: number; ids: HandleId[] }> = [];
  return {
    issue: () => ++issued,
    answer<T>(seq: number, build: (hold: (fn: (...args: any[]) => unknown) => HandleId) => T[]): T[] {
      if (disposed || kept.filter(reply => reply.seq > seq).length >= 2) return [];
      const ids: HandleId[] = [];
      let items: T[];
      try { items = build(fn => { const id = handle(fn); ids.push(id); return id; }); }
      catch (error) { for (const id of ids) release(id); throw error; }
      kept = [...kept, { seq, ids }].sort((a, b) => b.seq - a.seq);
      for (const reply of kept.splice(2)) for (const id of reply.ids) release(id);
      return items;
    },
    dispose() { disposed = true; for (const reply of kept.splice(0)) for (const id of reply.ids) release(id); }
  };
}
/* The runtime iframe is hidden, where a frame callback may never come; the timer bounds the wait. */
function nextFrame(fn: () => void): void {
  let done = false;
  const run = (): void => { if (!done) { done = true; fn(); } };
  if (typeof requestAnimationFrame === 'function') requestAnimationFrame(run);
  setTimeout(run, 50);
}
function deepFreeze<T>(value: T): T {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const item of Object.values(value)) deepFreeze(item);
  }
  return value;
}

/** A per-iframe API. Only serializable values and callback ids cross the port. */
export function createSandboxAPI(rpc: Rpc, init: SandboxInit, mode: SandboxMode = 'runtime'): PowermoveAPI {
  const state: SandboxState = { ...init.state, selection: deepFreeze(init.state.selection ?? null) };
  const theme = { active: init.theme.id ?? init.activeTheme ?? '', scheme: init.theme.scheme };
  /** True while a view's `activate` replays; see the mode comment above. */
  let quiet = false;
  const panelPorts = new Map<string, Rpc>();
  const disposers: Array<() => void> = [];
  const registrations = new Set<Promise<unknown>>();
  let registrationFailure: unknown;
  const vars = Object.freeze({ ...init.vars });
  const local = new Map<string, Map<string, any>>();
  let catalog = init.catalog;
  const list = (kind: string): any[] => {
    const merged = new Map((catalog?.[kind] ?? []).map(item => [item.id, item]));
    for (const [id, item] of local.get(kind) ?? []) merged.set(id, item);
    return [...merged.values()];
  };
  const easeRuntime: Record<string, any> = {};
  installEase(easeRuntime as unknown as PMRegistry);
  const clamp = (value: number, min: number, max: number): number => Math.max(min, Math.min(max, value));
  const util = {
    round: (value: number, places = 2) => Math.round(value * 10 ** places) / 10 ** places,
    clamp, lerp: (from: number, to: number, amount: number) => from + (to - from) * amount,
    snapF: (time: number, fps: number) => Math.round(time * fps) / fps,
    tc: (seconds: number, fps = 30, showFrames = true) => {
      let frame = Math.round(Math.abs(seconds) * fps);
      const ff = frame % fps; frame = Math.floor(frame / fps);
      const pad = (n: number) => String(n).padStart(2, '0');
      return `${seconds < 0 ? '-' : ''}${frame >= 3600 ? `${pad(Math.floor(frame / 3600))}:` : ''}${pad(Math.floor(frame / 60) % 60)}:${pad(frame % 60)}${showFrames ? `:${pad(ff)}` : ''}`;
    },
    parseTc: (value: string, fps = 30) => {
      const parts = String(value).trim().split(':').map(Number);
      if (parts.some(Number.isNaN)) return null;
      if (parts.length === 4) return (parts[0] ?? 0) * 3600 + (parts[1] ?? 0) * 60 + (parts[2] ?? 0) + (parts[3] ?? 0) / fps;
      if (parts.length === 3) return (parts[0] ?? 0) * 60 + (parts[1] ?? 0) + (parts[2] ?? 0) / fps;
      if (parts.length === 2) return (parts[0] ?? 0) + (parts[1] ?? 0) / fps;
      return parts[0] ?? null;
    },
    uid: (prefix = 'l') => `${prefix}${Math.random().toString(36).slice(2, 9)}`,
    hex2rgb: (hex: string) => { const text = hex.replace('#', ''); const full = text.length === 3 || text.length === 4 ? [...text].map(c => c + c).join('') : text; return [0, 2, 4].map(i => (parseInt(full.slice(i, i + 2), 16) || 0) / 255); },
    rgb2hex: (r: number, g: number, b: number) => `#${[r, g, b].map(v => clamp(Math.round(v * 255), 0, 255).toString(16).padStart(2, '0')).join('')}`
  };
  const report = sandboxReporter(rpc);
  const restricted = (name: string) => (..._args: unknown[]) => { report('permission', name); throw new PermissionError(name, 'project.apply or the pure matrix helpers'); };
  const send = (method: string, ...args: unknown[]): Promise<any> => {
    if (quiet && method === 'invoke' && !VIEW_READS.has(`${args[0]}.${args[1]}`)) return Promise.resolve(undefined);
    if (method === 'invoke' && !init.manifest.permissions?.includes('project:write')) {
      const [namespace, member, params] = args;
      const command = Array.isArray(params) ? params[0] : undefined;
      if (namespace === 'project' && member !== 'snapshot' || namespace === 'transport' ||
        namespace === 'commands' && typeof command === 'string' && !command.startsWith(`${init.id}.`) && !command.startsWith(`${init.id}-`))
        return Promise.reject(new ProjectWritePermissionError(`${String(namespace)}.${String(member)}`));
    }
    return rpc.call(method, ...args);
  };
  /* Methods that return a value synchronously in-realm and a Promise here.
     apiVersion 3 code awaits them; older code that reads the result at once
     gets undefined, and the sandbox check names the method. */
  const legacy = init.apiVersion < 3;
  const later = (member: string, method: string, ...args: unknown[]): Promise<any> => {
    const promise = send(method, ...args);
    return legacy ? watchPromise(promise, member, report) : promise;
  };
  const fire = (method: string, ...args: unknown[]): void => { void send(method, ...args).catch(error => {
    try { rpc.notify('runtime-error', { name: error.name, message: error.message, code: error.code }); }
    catch { /* port closed during teardown */ }
  }); };
  const registration = (method: string, value: unknown, handles: HandleId[] = [], localValue = value): { dispose(): void } => {
    const token = crypto.randomUUID();
    const item = localValue as Record<string, any>;
    if (typeof item?.id === 'string') {
      let entries = local.get(method);
      if (!entries) { entries = new Map(); local.set(method, entries); }
      entries.set(item.id, item);
    }
    /* A view records registrations for its own lookup only; the runtime owns
       the kernel's copy. Event subscriptions are the exception: they are how
       panel UI hears about the project. */
    const forwarded = mode === 'runtime' || method === 'events';
    if (forwarded) {
      const pending = send('register', method, token, value);
      registrations.add(pending);
      void pending.then(() => registrations.delete(pending), error => { registrations.delete(pending); registrationFailure ??= error; });
    }
    let disposed = false;
    const dispose = (): void => {
      if (disposed) return; disposed = true;
      if (forwarded) fire('dispose-registration', token);
      if (typeof item?.id === 'string') local.get(method)?.delete(item.id);
      for (const handle of handles) rpc.release(handle);
    };
    disposers.push(dispose);
    return { dispose };
  };
  const handle = (fn: (...args: any[]) => unknown): HandleId => rpc.handle(fn);
  /* Callbacks cannot cross the port, so a toast's `action.run` and
     `onDismiss` go as handles. The host releases them when the toast closes,
     however it closes; one the host refuses releases them here. */
  const toast = (message: string, options?: unknown): void => {
    if (!options || typeof options !== 'object') { fire('invoke', 'ui', 'toast', options === undefined ? [message] : [message, options]); return; }
    if (quiet) return;
    const input = options as Record<string, any>;
    const value: Record<string, unknown> = {};
    for (const [key, item] of Object.entries(input)) if (key !== 'action' && key !== 'onDismiss' && typeof item !== 'function') value[key] = item;
    const ids: HandleId[] = [];
    const release = (): void => { for (const id of ids) { try { rpc.release(id); } catch { /* port closed */ } } };
    try {
      if (input.action && typeof input.action.run === 'function') { const run = handle(input.action.run); ids.push(run); value.action = { label: input.action.label, run }; }
      if (typeof input.onDismiss === 'function') { const dismiss = handle(input.onDismiss); ids.push(dismiss); value.onDismiss = dismiss; }
    } catch (error) { release(); throw error; }
    void send('invoke', 'ui', 'toast', [message, value]).catch(error => { release(); raise(error); });
  };
  /* The parsed snapshot of the newest generation fetched; see the data plane comment above. */
  let snapshot: { generation: number; value?: unknown; tooLarge?: true } | null = null;
  let inflight: Promise<void> | null = null;
  const fetchSnapshot = (): Promise<void> => inflight ??= (rpc.call('project-snapshot') as Promise<SandboxSnapshot>).then(reply => {
    // `unchanged` names the generation already held here: the copy stands.
    if ('unchanged' in reply || snapshot && reply.generation < snapshot.generation) return;
    snapshot = 'json' in reply ? { generation: reply.generation, value: deepFreeze(JSON.parse(reply.json)) } : { generation: reply.generation, tooLarge: true };
    reads.project.bump();
  }).finally(() => { inflight = null; });
  const pullProject = async (): Promise<unknown> => {
    const wanted = state.generation;
    /* A reply the kernel built before the change this document already heard
       about is one generation behind; the next call is not. */
    for (let attempt = 0; attempt < 3 && !(snapshot && snapshot.generation >= wanted); attempt++) await fetchSnapshot();
    if (!snapshot) throw new Error('project.get() could not read the project');
    if (snapshot.tooLarge) throw new Error('The project is larger than the 8 Mi character sandbox snapshot limit, so project.get() is unavailable until it shrinks');
    return snapshot.value;
  };
  /* A `latest()` reader keeps this document's copy at the current generation.
     A drag changes the project on every pointer move, and each pull builds a
     whole-project copy on the host, so pulls wait for the one in flight and
     for the next frame: at most one a frame, the newest change always among
     them. A failed pull leaves what `latest()` returns as it was. */
  let latestBusy = false, latestWanted = false;
  const pullLatest = (): void => {
    if (snapshot && snapshot.generation >= state.generation) return;
    if (latestBusy) { latestWanted = true; return; }
    latestBusy = true;
    void Promise.all([pullProject().catch(() => {}), new Promise<void>(nextFrame)]).then(() => {
      latestBusy = false;
      if (!latestWanted) return;
      latestWanted = false;
      if (reads.project.live()) pullLatest();
    });
  };
  const reads = { time: reactiveSource(), playing: reactiveSource(), revision: reactiveSource(), selection: reactiveSource(),
    project: reactiveSource(pullLatest), theme: reactiveSource() };
  const reactive = <T,>(source: { read(): void }, value: () => T) => (): T => { source.read(); return value(); };
  const raise = (error: unknown): void => { try { rpc.notify('runtime-error', serializeRpcError(error)); } catch { /* port closed */ } };
  /* Listeners live here; the kernel only learns which names have one. */
  const listeners = new Map<string, { fns: Set<{ fn: (payload: unknown) => unknown }>; interest: { dispose(): void } }>();
  const listen = (event: string, fn: (payload: any) => unknown): { dispose(): void } => {
    const name = String(event);
    if (typeof fn !== 'function') throw new TypeError('events.on needs a listener function');
    let entry = listeners.get(name);
    if (!entry) { entry = { fns: new Set(), interest: registration('events', { event: name }) }; listeners.set(name, entry); }
    const listener = { fn };
    entry.fns.add(listener);
    const owner = entry;
    let disposed = false;
    const dispose = (): void => {
      if (disposed) return; disposed = true;
      owner.fns.delete(listener);
      if (owner.fns.size || listeners.get(name) !== owner) return;
      listeners.delete(name); owner.interest.dispose();
    };
    disposers.push(dispose);
    return { dispose };
  };
  const simpleRegister = (namespace: string) => (definition: unknown) => registration(namespace, definition);
  const menuContributors = new Set<{ location: string; fn: (...args: any[]) => unknown }>();
  const api: Record<string, any> = {
    id: init.id, apiVersion: init.apiVersion, manifest: init.manifest,
    effects: { supportsVersions: true, supportsSpatialSampling: true, supportsLayerInputs: true, supportsInputSubframes: true, supportsSourceRasters: true, supportsSourceOwners: true, supportsBrushGeometry: true, supportsOwnerValidity: true, supportsLutAssets: true, supportsTypedReferences: true, parameterDefault: effectParameterDefault, register: simpleRegister('effects'), list: () => list('effects'), get: (id: string, version?: number) => resolveEffectDefinition(list('effects').find(item => item.id === id) as any, version) },
    transitions: { register: simpleRegister('transitions'), list: () => list('transitions'), get: (id: string) => list('transitions').find(item => item.id === id) },
    layers: { register: simpleRegister('layers'), list: () => list('layers'), get: (id: string) => list('layers').find(item => item.id === id) },
    theme: { register: simpleRegister('theme'), activate: (id: string) => fire('invoke', 'theme', 'activate', [id]), setScheme: () => { report('permission', 'theme.setScheme'); throw new PermissionError('theme.setScheme'); }, active: reactive(reads.theme, () => theme.active), scheme: reactive(reads.theme, () => theme.scheme), list: () => list('theme') },
    keybindings: { bind: simpleRegister('keybindings'), unbind: (key: string) => fire('invoke', 'keybindings', 'unbind', [key]), list: () => list('keybindings'), chordOf: (event: KeyboardEvent) => {
      const parts = [event.metaKey && 'cmd', event.ctrlKey && 'ctrl', event.altKey && 'alt', event.shiftKey && 'shift', event.key.toLowerCase()].filter(Boolean);
      return parts.join('+');
    } },
    commands: { register(def: Record<string, any>) {
      if (mode === 'view') return registration('commands', def, [], def);
      const ids = [handle(def.run)];
      const value: Record<string, any> = { ...def, run: ids[0] };
      if (def.when) { ids.push(handle(def.when)); value.when = ids[1]; }
      return registration('commands', value, ids, def);
    }, run: (id: string, ...args: unknown[]) => later('commands.run', 'invoke', 'commands', 'run', [id, ...args]), has: (id: string) => list('commands').some(item => item.id === id), list: () => list('commands') },
    status: { register(def: Record<string, any>) {
      if (mode === 'view') return registration('status', def, [], def);
      const ids = [handle(def.text)]; const value: Record<string, any> = { ...def, text: ids[0] };
      if (def.onClick) { ids.push(handle(def.onClick)); value.onClick = ids[1]; }
      return registration('status', value, ids, def);
    }, list: () => list('status') },
    palette: { registerProvider(fn: (...args: any[]) => unknown) {
      if (mode === 'view') return registration('palette', { provider: 0 }, [], fn);
      const replies = replyHandles(handle, id => rpc.release(id));
      // The entries may be a Promise.
      const id = handle(async (query: string) => {
        const seq = replies.issue();
        const entries = await fn(query) as Record<string, any>[];
        return replies.answer(seq, hold => entries.map(entry => ({ ...entry, run: hold(entry.run) })));
      });
      const registrationHandle = registration('palette', { provider: id }, [id]);
      const disposable = { dispose() { registrationHandle.dispose(); replies.dispose(); } };
      disposers.push(disposable.dispose);
      return disposable;
    }, open: (query?: string) => fire('invoke', 'palette', 'open', query === undefined ? [] : [query]) },
    menus: { contribute(location: string, fn: (...args: any[]) => unknown) {
      const own = { location, fn };
      menuContributors.add(own);
      if (mode === 'view') { const recorded = registration('menus', { location, items: 0 }, [], fn); return { dispose() { menuContributors.delete(own); recorded.dispose(); } }; }
      const replies = replyHandles(handle, id => rpc.release(id));
      // The items may be a Promise; the host waits for them within its menu deadline.
      const id = handle(async (ctx: unknown) => {
        const seq = replies.issue();
        const items = await fn(ctx) as Array<string | Record<string, any>>;
        return replies.answer(seq, hold => items.map(entry => typeof entry === 'string' || !entry.run ? entry : { ...entry, run: hold(entry.run) }));
      });
      const registrationHandle = registration('menus', { location, items: id }, [id]);
      const disposable = { dispose() { menuContributors.delete(own); registrationHandle.dispose(); replies.dispose(); } };
      disposers.push(disposable.dispose);
      return disposable;
    /* Only this extension's own contributions: another's items carry
       callbacks that belong to its document. A throwing contributor is
       reported and skipped, as in-realm. */
    }, collect: (location: string, ctx: Record<string, unknown> = {}) => [...menuContributors].flatMap(({ location: where, fn }) => {
      if (where !== location) return [];
      try { const items = fn(ctx); return Array.isArray(items) ? items : []; } catch (error) { raise(error); return []; }
    }), gather: async (location: string, ctx: Record<string, unknown> = {}) => (await Promise.all([...menuContributors].filter(({ location: where }) => where === location).map(async ({ fn }) => {
      try { const items = await fn(ctx); return Array.isArray(items) ? items : []; } catch (error) { raise(error); return []; }
    }))).flat() },
    panels: { register(def: Record<string, any>) {
      if (!def || typeof def.id !== 'string' || !def.id) throw new Error('panels.register requires an id');
      if (!def.component && typeof def.build !== 'function') throw new Error(`panel "${def.id}" needs component or build`);
      /* Only layout hints cross. `header`, `moveSlot`, `headless` and
         `library.render` touch the app's DOM and do not exist for sandboxed
         panels: the host draws the header from title, and the Library shows
         the icon on the extension's art. */
      return registration('panels', panelInfo(def), [], def);
    }, list: () => list('panels').map(item => item.id), open: (id: string, options?: unknown) => fire('invoke', 'panels', 'open', options === undefined ? [id] : [id, options]), close: (id: string) => fire('invoke', 'panels', 'close', [id]), refresh: (id: string) => fire('invoke', 'panels', 'refresh', [id]), isOpen: (id: string) => later('panels.isOpen', 'invoke', 'panels', 'isOpen', [id]) },
    project: { get: () => legacy ? watchPromise(pullProject(), 'project.get', report) : pullProject(),
      latest: reactive(reads.project, () => snapshot?.value), revision: reactive(reads.revision, () => state.revision), selection: reactive(reads.selection, () => state.selection),
      time: reactive(reads.time, () => state.time), playing: reactive(reads.playing, () => state.playing),
      apply: (...args: unknown[]) => later('project.apply', 'invoke', 'project', 'apply', args), select: (...args: unknown[]) => later('project.select', 'invoke', 'project', 'select', args),
      setTime: (time: number) => later('project.setTime', 'invoke', 'project', 'setTime', [time]), play: () => later('project.play', 'invoke', 'project', 'play', []), pause: () => later('project.pause', 'invoke', 'project', 'pause', []), undo: () => later('project.undo', 'invoke', 'project', 'undo', []), redo: () => later('project.redo', 'invoke', 'project', 'redo', []), snapshot: (...args: unknown[]) => send('invoke', 'project', 'snapshot', args) },
    transport: { time: reactive(reads.time, () => state.time), playing: reactive(reads.playing, () => state.playing), setTime: (time: number) => later('transport.setTime', 'invoke', 'project', 'setTime', [time]), play: () => later('transport.play', 'invoke', 'project', 'play', []), pause: () => later('transport.pause', 'invoke', 'project', 'pause', []), toggle: () => later('transport.toggle', 'invoke', 'project', state.playing ? 'pause' : 'play', []), step: (frames: number) => later('transport.step', 'invoke', 'transport', 'step', [frames]) },
    assets: { pick: (...args: unknown[]) => send('invoke', 'assets', 'pick', args), import: (...args: unknown[]) => send('invoke', 'assets', 'import', args), importUrl: (url: string) => send('invoke', 'assets', 'importUrl', [url]), get: (id: string) => later('assets.get', 'invoke', 'assets', 'get', [id]), readText: (id: string) => send('invoke', 'assets', 'readText', [id]) },
    storage: { get: (key: string) => later('storage.get', 'invoke', 'storage', 'get', [key]), set: (key: string, value: unknown) => later('storage.set', 'invoke', 'storage', 'set', [key, value]), delete: (key: string) => later('storage.delete', 'invoke', 'storage', 'delete', [key]) },
    media: partlyTrusted('media', { registerImportDefaults: simpleRegister('media-defaults'), getImportDefaults: () => later('media.getImportDefaults', 'invoke', 'media', 'getImportDefaults', []) }, report),
    events: { on: listen, emit: (event: string, payload: unknown) => fire('invoke', 'events', 'emit', [event, payload]) },
    ui: partlyTrusted('ui', { toast, confirm: (...args: unknown[]) => send('invoke', 'ui', 'confirm', args), openExternal: (url: string) => send('invoke', 'ui', 'openExternal', [url]), icon: (...args: unknown[]) => later('ui.icon', 'invoke', 'ui', 'icon', args), copy: (text: string) => send('invoke', 'ui', 'copy', [text]), controls: trustedOnly('ui.controls', report), modal: trustedOnly('ui.modal', report), menu: trustedOnly('ui.menu', report), drag: trustedOnly('ui.drag', report), gesture: trustedOnly('ui.gesture', report), mount: trustedOnly('ui.mount', report) }, report),
    vars: { get: (key: string) => vars[key], has: (key: string) => Object.hasOwn(vars, key), keys: () => Object.keys(vars) },
    extensions: { list: () => later('extensions.list', 'extensions-list'), setUp: (id: string) => send('invoke', 'extensions', 'setUp', [id]),
      fork: restricted('extensions.fork'), setEnabled: restricted('extensions.setEnabled'), remove: restricted('extensions.remove'), reload: restricted('extensions.reload'), reveal: restricted('extensions.reveal'), requestFix: restricted('extensions.requestFix'), rebase: restricted('extensions.rebase') },
    log: (level: string, message: string, ...data: unknown[]) => rpc.notify('log', level, message, data),
    onDispose: (fn: () => void) => { disposers.push(fn); }
  };
  for (const name of ['anim', 'model', 'selection', 'groups', 'history', 'edit', 'inspector', 'render', 'uiState', 'dnd', 'workspace', 'services', 'host']) api[name] = trustedOnly(name, report);
  api.util = util;
  api.ease = easeRuntime.Ease;
  api.space3d = { CHANNELS_3D, projectPoint, inversePlane,
    local3D: restricted('space3d.local3D'), parent3D: restricted('space3d.parent3D'),
    world3D: restricted('space3d.world3D'), is3DLayer: restricted('space3d.is3DLayer'),
    perspectiveAmount: restricted('space3d.perspectiveAmount'), planeMatrix: restricted('space3d.planeMatrix'),
    planeContains: restricted('space3d.planeContains') };
  api.on = api.events.on;
  const control: SandboxControl = {
    tick(delta: Partial<SandboxState>, events: SandboxEvent[]) {
      if (delta) {
        Object.assign(state, delta);
        if ('selection' in delta) state.selection = deepFreeze(delta.selection ?? null);
        for (const key of ['time', 'playing', 'revision', 'selection'] as const) if (key in delta) reads[key].bump();
        if ('generation' in delta && reads.project.live()) pullLatest();
      }
      for (const [name, payload] of events ?? []) {
        const entry = listeners.get(name);
        if (entry) for (const { fn } of [...entry.fns]) { try { void fn(payload); } catch (error) { raise(error); } }
      }
    },
    theme(next) {
      const active = next.id ?? theme.active;
      if (active === theme.active && next.scheme === theme.scheme) return;
      theme.active = active; theme.scheme = next.scheme;
      reads.theme.bump();
    },
    catalog(next) { if (next && typeof next === 'object') catalog = next; },
    ready: async () => { await Promise.all([...registrations]); if (registrationFailure) throw registrationFailure; },
    dispose() {
      for (const port of panelPorts.values()) port.close();
      panelPorts.clear();
      for (const fn of disposers.reverse()) fn();
    },
    setQuiet(on: boolean) { quiet = on; },
    panel: (id: string) => local.get('panels')?.get(id),
    /* Runtime side of the brokered port: the kernel hands the runtime one end
       and the panel's view iframe the other, so they talk directly. */
    mountPanel(panelId: string, token: string, port: MessagePort) {
      panelPorts.get(token)?.close();
      panelPorts.set(token, createRpc(port, {
        definition(id: string) {
          if (id !== panelId) return null;
          const def = local.get('panels')?.get(id);
          return def ? { ...panelInfo(def), kind: def.component ? 'component' : 'build' } : null;
        }
      }));
    },
    unmountPanel(token: string) { panelPorts.get(token)?.close(); panelPorts.delete(token); }
  };
  Object.defineProperty(api, '__sandbox', { value: control });
  return api as PowermoveAPI;
}
