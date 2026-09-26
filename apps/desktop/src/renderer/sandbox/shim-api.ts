import type { PowermoveAPI } from '../src/kernel/api';
import type { PMRegistry } from '../src/legacy/registry';
import { createRpc, type Rpc, type HandleId } from '../../shared/sandbox-rpc';
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
  update(next: SandboxMirror): void;
  ready(): Promise<unknown>;
  dispose(): void;
  setQuiet(on: boolean): void;
  panel(id: string): Record<string, any> | undefined;
  mountPanel(panelId: string, token: string, port: MessagePort): void;
  unmountPanel(token: string): void;
}
export const sandboxControl = (api: PowermoveAPI): SandboxControl => (api as unknown as { __sandbox: SandboxControl }).__sandbox;
export interface SandboxMirror { project: unknown; revision: number; selection: unknown; time: number; playing: boolean }
export interface SandboxInit {
  id: string; apiVersion: number; manifest: PowermoveAPI['manifest']; vars: Record<string, string>;
  theme: { scheme: string; tokens: Record<string, string> }; project: SandboxMirror; bundleUrl: string;
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
 *   - `events.on` subscriptions are forwarded, so panel UI stays live;
 *   - reads (project mirror, vars, storage.get, extensions.list, ...) work;
 *   - while the view's `activate` runs, side-effecting calls (panels.open,
 *     toasts, storage.set, project edits, events.emit, ...) are dropped,
 *     because the runtime's own `activate` already performed them once.
 */
export type SandboxMode = 'runtime' | 'view';
const VIEW_READS = new Set(['storage.get', 'assets.get', 'assets.readText', 'media.getImportDefaults', 'ui.icon']);

/** A per-iframe API. Only serializable values and callback ids cross the port. */
export function createSandboxAPI(rpc: Rpc, init: SandboxInit, mode: SandboxMode = 'runtime'): PowermoveAPI {
  let mirror = init.project;
  /** True while a view's `activate` replays; see the mode comment above. */
  let quiet = false;
  const panelPorts = new Map<string, Rpc>();
  const disposers: Array<() => void> = [];
  const registrations = new Set<Promise<unknown>>();
  let registrationFailure: unknown;
  const vars = Object.freeze({ ...init.vars });
  const local = new Map<string, Map<string, any>>();
  const list = (kind: string): any[] => {
    const merged = new Map((init.catalog?.[kind] ?? []).map(item => [item.id, item]));
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
  const simpleRegister = (namespace: string) => (definition: unknown) => registration(namespace, definition);
  const api: Record<string, any> = {
    id: init.id, apiVersion: init.apiVersion, manifest: init.manifest,
    effects: { register: simpleRegister('effects'), list: () => list('effects'), get: (id: string) => list('effects').find(item => item.id === id) },
    transitions: { register: simpleRegister('transitions'), list: () => list('transitions'), get: (id: string) => list('transitions').find(item => item.id === id) },
    layers: { register: simpleRegister('layers'), list: () => list('layers'), get: (id: string) => list('layers').find(item => item.id === id) },
    theme: { register: simpleRegister('theme'), activate: (id: string) => fire('invoke', 'theme', 'activate', [id]), setScheme: () => { report('permission', 'theme.setScheme'); throw new PermissionError('theme.setScheme'); }, active: () => init.activeTheme ?? '', scheme: () => init.theme.scheme, list: () => list('theme') },
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
      let current: HandleId[] = [], previous: HandleId[] = [];
      const id = handle((query: string) => {
        for (const item of previous) rpc.release(item);
        previous = current; current = [];
        return (fn(query) as Record<string, any>[]).map(entry => {
          const run = handle(entry.run); current.push(run); return { ...entry, run };
        });
      });
      const registrationHandle = registration('palette', { provider: id }, [id]);
      const disposable = { dispose() { registrationHandle.dispose(); for (const item of [...previous, ...current]) rpc.release(item); previous = []; current = []; } };
      disposers.push(disposable.dispose);
      return disposable;
    }, open: (query?: string) => fire('invoke', 'palette', 'open', query === undefined ? [] : [query]) },
    menus: { contribute(location: string, fn: (...args: any[]) => unknown) {
      if (mode === 'view') return registration('menus', { location, items: 0 }, [], fn);
      let current: HandleId[] = [], previous: HandleId[] = [];
      const id = handle((ctx: unknown) => {
        for (const item of previous) rpc.release(item);
        previous = current; current = [];
        return (fn(ctx) as Array<string | Record<string, any>>).map(entry => {
          if (typeof entry === 'string' || !entry.run) return entry;
          const run = handle(entry.run); current.push(run); return { ...entry, run };
        });
      });
      const registrationHandle = registration('menus', { location, items: id }, [id]);
      const disposable = { dispose() { registrationHandle.dispose(); for (const item of [...previous, ...current]) rpc.release(item); previous = []; current = []; } };
      disposers.push(disposable.dispose);
      return disposable;
    }, collect: () => [] },
    panels: { register(def: Record<string, any>) {
      if (!def || typeof def.id !== 'string' || !def.id) throw new Error('panels.register requires an id');
      if (!def.component && typeof def.build !== 'function') throw new Error(`panel "${def.id}" needs component or build`);
      /* Only layout hints cross. `header`, `moveSlot`, `headless` and
         `library.render` touch the app's DOM and do not exist for sandboxed
         panels: the host draws the header from title, and the Library shows
         the icon on the extension's art. */
      return registration('panels', panelInfo(def), [], def);
    }, list: () => list('panels').map(item => item.id), open: (id: string, options?: unknown) => fire('invoke', 'panels', 'open', options === undefined ? [id] : [id, options]), close: (id: string) => fire('invoke', 'panels', 'close', [id]), refresh: (id: string) => fire('invoke', 'panels', 'refresh', [id]), isOpen: () => false },
    project: { get: () => { if (mirror.project && typeof mirror.project === 'object' && 'tooLarge' in mirror.project) throw new Error('Project mirror exceeds 8 MiB; project.get() is unavailable at this revision'); return mirror.project; }, revision: () => mirror.revision, selection: () => mirror.selection,
      time: () => mirror.time, playing: () => mirror.playing,
      apply: (...args: unknown[]) => later('project.apply', 'invoke', 'project', 'apply', args), select: (...args: unknown[]) => later('project.select', 'invoke', 'project', 'select', args),
      setTime: (time: number) => later('project.setTime', 'invoke', 'project', 'setTime', [time]), play: () => later('project.play', 'invoke', 'project', 'play', []), pause: () => later('project.pause', 'invoke', 'project', 'pause', []), undo: () => later('project.undo', 'invoke', 'project', 'undo', []), redo: () => later('project.redo', 'invoke', 'project', 'redo', []), snapshot: (...args: unknown[]) => send('invoke', 'project', 'snapshot', args) },
    transport: { time: () => mirror.time, playing: () => mirror.playing, setTime: (time: number) => later('transport.setTime', 'invoke', 'project', 'setTime', [time]), play: () => later('transport.play', 'invoke', 'project', 'play', []), pause: () => later('transport.pause', 'invoke', 'project', 'pause', []), toggle: () => later('transport.toggle', 'invoke', 'project', mirror.playing ? 'pause' : 'play', []), step: (frames: number) => later('transport.step', 'invoke', 'transport', 'step', [frames]) },
    assets: { pick: (...args: unknown[]) => send('invoke', 'assets', 'pick', args), import: (...args: unknown[]) => send('invoke', 'assets', 'import', args), get: (id: string) => later('assets.get', 'invoke', 'assets', 'get', [id]), readText: (id: string) => send('invoke', 'assets', 'readText', [id]) },
    storage: { get: (key: string) => later('storage.get', 'invoke', 'storage', 'get', [key]), set: (key: string, value: unknown) => later('storage.set', 'invoke', 'storage', 'set', [key, value]), delete: (key: string) => later('storage.delete', 'invoke', 'storage', 'delete', [key]) },
    media: partlyTrusted('media', { registerImportDefaults: simpleRegister('media-defaults'), getImportDefaults: () => later('media.getImportDefaults', 'invoke', 'media', 'getImportDefaults', []) }, report),
    events: { on(event: string, fn: (...args: any[]) => unknown) { const id = handle(fn); return registration('events', { event, fn: id }, [id]); }, emit: (event: string, payload: unknown) => fire('invoke', 'events', 'emit', [event, payload]) },
    ui: partlyTrusted('ui', { toast: (message: string, options?: unknown) => fire('invoke', 'ui', 'toast', options === undefined ? [message] : [message, options]), confirm: (...args: unknown[]) => send('invoke', 'ui', 'confirm', args), icon: (...args: unknown[]) => later('ui.icon', 'invoke', 'ui', 'icon', args), controls: trustedOnly('ui.controls', report), modal: trustedOnly('ui.modal', report), menu: trustedOnly('ui.menu', report), drag: trustedOnly('ui.drag', report), gesture: trustedOnly('ui.gesture', report), mount: trustedOnly('ui.mount', report) }, report),
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
    update(next: SandboxMirror) { mirror = next; },
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
