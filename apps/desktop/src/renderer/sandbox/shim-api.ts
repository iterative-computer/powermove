import type { PowermoveAPI } from '../src/kernel/api';
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
const trustedOnly = (name: string, alternative?: string): any => new Proxy({}, {
  get(_target, member) {
    if (member === 'then') return undefined;
    throw new PermissionError(`${name}.${String(member)}`, alternative);
  }
});
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
const restricted = (name: string) => (..._args: unknown[]) => { throw new PermissionError(name, 'project.apply or the pure matrix helpers'); };

/** A per-iframe API. Only serializable values and callback ids cross the port. */
export function createSandboxAPI(rpc: Rpc, init: SandboxInit, mode: SandboxMode = 'runtime'): PowermoveAPI {
  let mirror = init.project;
  /** True while a view's `activate` replays; see the mode comment above. */
  let quiet = false;
  const panelPorts = new Map<string, Rpc>();
  const disposers: Array<() => void> = [];
  const registrations = new Set<Promise<unknown>>();
  const vars = Object.freeze({ ...init.vars });
  const local = new Map<string, Map<string, any>>();
  const list = (kind: string): any[] => {
    const merged = new Map((init.catalog?.[kind] ?? []).map(item => [item.id, item]));
    for (const [id, item] of local.get(kind) ?? []) merged.set(id, item);
    return [...merged.values()];
  };
  const easeRuntime: Record<string, any> = {};
  installEase(easeRuntime as any);
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
      if (parts.length === 4) return parts[0]! * 3600 + parts[1]! * 60 + parts[2]! + parts[3]! / fps;
      if (parts.length === 3) return parts[0]! * 60 + parts[1]! + parts[2]! / fps;
      if (parts.length === 2) return parts[0]! + parts[1]! / fps;
      return parts[0] ?? null;
    },
    uid: (prefix = 'l') => `${prefix}${Math.random().toString(36).slice(2, 9)}`,
    hex2rgb: (hex: string) => { const text = hex.replace('#', ''); const full = text.length === 3 ? [...text].map(c => c + c).join('') : text; const value = parseInt(full, 16); return [((value >> 16) & 255) / 255, ((value >> 8) & 255) / 255, (value & 255) / 255]; },
    rgb2hex: (r: number, g: number, b: number) => `#${[r, g, b].map(v => clamp(Math.round(v * 255), 0, 255).toString(16).padStart(2, '0')).join('')}`
  };
  const send = (method: string, ...args: unknown[]): Promise<any> => {
    if (quiet && method === 'invoke' && !VIEW_READS.has(`${args[0]}.${args[1]}`)) return Promise.resolve(undefined);
    return rpc.call(method, ...args);
  };
  const fire = (method: string, ...args: unknown[]): void => { void send(method, ...args).catch(error => rpc.notify('runtime-error', { name: error.name, message: error.message, code: error.code })); };
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
      void pending.finally(() => registrations.delete(pending)).catch(() => {});
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
    theme: { register: simpleRegister('theme'), activate: (id: string) => fire('invoke', 'theme', 'activate', [id]), setScheme: (value: string) => fire('invoke', 'theme', 'setScheme', [value]), active: () => init.activeTheme ?? '', scheme: () => init.theme.scheme, list: () => list('theme') },
    keybindings: { bind: simpleRegister('keybindings'), unbind: (key: string, all?: boolean) => fire('invoke', 'keybindings', 'unbind', [key, all]), list: () => list('keybindings'), chordOf: (event: KeyboardEvent) => {
      const parts = [event.metaKey && 'cmd', event.ctrlKey && 'ctrl', event.altKey && 'alt', event.shiftKey && 'shift', event.key.toLowerCase()].filter(Boolean);
      return parts.join('+');
    } },
    commands: { register(def: Record<string, any>) {
      const ids = [handle(def.run)];
      const value: Record<string, any> = { ...def, run: ids[0] };
      if (def.when) { ids.push(handle(def.when)); value.when = ids[1]; }
      return registration('commands', value, ids, def);
    }, run: (id: string, ...args: unknown[]) => send('invoke', 'commands', 'run', [id, ...args]), has: (id: string) => list('commands').some(item => item.id === id), list: () => list('commands') },
    status: { register(def: Record<string, any>) {
      const ids = [handle(def.text)]; const value: Record<string, any> = { ...def, text: ids[0] };
      if (def.onClick) { ids.push(handle(def.onClick)); value.onClick = ids[1]; }
      return registration('status', value, ids, def);
    }, list: () => list('status') },
    palette: { registerProvider(fn: (...args: any[]) => unknown) {
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
    }, open: (query?: string) => fire('invoke', 'palette', 'open', [query]) },
    menus: { contribute(location: string, fn: (...args: any[]) => unknown) {
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
    }, list: () => list('panels').map(item => item.id), open: (id: string, options?: unknown) => fire('invoke', 'panels', 'open', [id, options]), close: (id: string) => fire('invoke', 'panels', 'close', [id]), refresh: (id: string) => fire('invoke', 'panels', 'refresh', [id]), isOpen: () => false },
    project: { get: () => mirror.project, revision: () => mirror.revision, selection: () => mirror.selection,
      time: () => mirror.time, playing: () => mirror.playing,
      apply: (...args: unknown[]) => send('invoke', 'project', 'apply', args), select: (...args: unknown[]) => send('invoke', 'project', 'select', args),
      setTime: (time: number) => send('invoke', 'project', 'setTime', [time]), play: () => send('invoke', 'project', 'play', []), pause: () => send('invoke', 'project', 'pause', []), undo: () => send('invoke', 'project', 'undo', []), redo: () => send('invoke', 'project', 'redo', []), snapshot: (...args: unknown[]) => send('invoke', 'project', 'snapshot', args) },
    transport: { time: () => mirror.time, playing: () => mirror.playing, setTime: (time: number) => send('invoke', 'project', 'setTime', [time]), play: () => send('invoke', 'project', 'play', []), pause: () => send('invoke', 'project', 'pause', []), toggle: () => send('invoke', 'project', mirror.playing ? 'pause' : 'play', []), step: (frames: number) => send('invoke', 'transport', 'step', [frames]) },
    assets: { pick: (...args: unknown[]) => send('invoke', 'assets', 'pick', args), import: (...args: unknown[]) => send('invoke', 'assets', 'import', args), get: (id: string) => send('invoke', 'assets', 'get', [id]), readText: (id: string) => send('invoke', 'assets', 'readText', [id]) },
    storage: { get: (key: string) => send('invoke', 'storage', 'get', [key]), set: (key: string, value: unknown) => send('invoke', 'storage', 'set', [key, value]), delete: (key: string) => send('invoke', 'storage', 'delete', [key]) },
    media: { registerImportDefaults: simpleRegister('media-defaults'), getImportDefaults: () => send('invoke', 'media', 'getImportDefaults', []) },
    events: { on(event: string, fn: (...args: any[]) => unknown) { const id = handle(fn); return registration('events', { event, fn: id }, [id]); }, emit: (event: string, payload: unknown) => fire('invoke', 'events', 'emit', [event, payload]) },
    ui: { toast: (message: string, options?: unknown) => fire('invoke', 'ui', 'toast', [message, options]), confirm: (...args: unknown[]) => send('invoke', 'ui', 'confirm', args), icon: (...args: unknown[]) => send('invoke', 'ui', 'icon', args), controls: trustedOnly('ui.controls'), modal: trustedOnly('ui.modal'), menu: trustedOnly('ui.menu'), drag: trustedOnly('ui.drag'), gesture: trustedOnly('ui.gesture'), mount: trustedOnly('ui.mount') },
    vars: { get: (key: string) => vars[key], has: (key: string) => Object.hasOwn(vars, key), keys: () => Object.keys(vars) },
    extensions: { list: () => send('extensions-list'), fork: () => { throw new PermissionError('extensions.fork'); }, setEnabled: (...args: unknown[]) => send('invoke', 'extensions', 'setEnabled', args), remove: (...args: unknown[]) => send('invoke', 'extensions', 'remove', args), reload: (...args: unknown[]) => send('invoke', 'extensions', 'reload', args), reveal: (...args: unknown[]) => send('invoke', 'extensions', 'reveal', args) },
    log: (level: string, message: string, ...data: unknown[]) => rpc.notify('log', level, message, data),
    onDispose: (fn: () => void) => { disposers.push(fn); }
  };
  for (const name of ['anim', 'model', 'selection', 'groups', 'history', 'edit', 'inspector', 'render', 'uiState', 'dnd', 'workspace', 'services', 'host']) api[name] = trustedOnly(name);
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
    ready: () => Promise.all([...registrations]),
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
