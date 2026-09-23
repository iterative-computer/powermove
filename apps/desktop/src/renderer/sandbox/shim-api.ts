import type { PowermoveAPI } from '../src/kernel/api';
import type { Rpc, HandleId } from '../../shared/sandbox-rpc';
import { install as installEase } from '../src/legacy/core/easing';
import { CHANNELS_3D, projectPoint, inversePlane } from '../src/legacy/core/space-3d';

export class PermissionError extends Error {
  readonly code = 'full-access';
  constructor(member: string, alternative = 'project.apply or commands') {
    super(`${member} requires full access. Use ${alternative} in a sandboxed extension.`);
    this.name = 'PermissionError';
  }
}
export interface SandboxMirror { project: unknown; revision: number; selection: unknown; time: number; playing: boolean }
export interface SandboxInit {
  id: string; apiVersion: number; manifest: PowermoveAPI['manifest']; vars: Record<string, string>;
  theme: { scheme: string; tokens: Record<string, string> }; project: SandboxMirror; bundleUrl: string;
  catalog?: Record<string, Array<Record<string, any>>>;
  activeTheme?: string;
}
const trustedOnly = (name: string, alternative?: string): any => new Proxy({}, {
  get(_target, member) {
    if (member === 'then') return undefined;
    throw new PermissionError(`${name}.${String(member)}`, alternative);
  }
});
const restricted = (name: string) => (..._args: unknown[]) => { throw new PermissionError(name, 'project.apply or the pure matrix helpers'); };

/** A per-iframe API. Only serializable values and callback ids cross the port. */
export function createSandboxAPI(rpc: Rpc, init: SandboxInit): PowermoveAPI {
  let mirror = init.project;
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
  const send = (method: string, ...args: unknown[]): Promise<any> => rpc.call(method, ...args);
  const fire = (method: string, ...args: unknown[]): void => { void send(method, ...args).catch(error => rpc.notify('runtime-error', { name: error.name, message: error.message, code: error.code })); };
  const registration = (method: string, value: unknown, handles: HandleId[] = [], localValue = value): { dispose(): void } => {
    const token = crypto.randomUUID();
    const item = localValue as Record<string, any>;
    if (typeof item?.id === 'string') {
      let entries = local.get(method);
      if (!entries) { entries = new Map(); local.set(method, entries); }
      entries.set(item.id, item);
    }
    const pending = send('register', method, token, value);
    registrations.add(pending);
    void pending.finally(() => registrations.delete(pending)).catch(() => {});
    let disposed = false;
    const dispose = (): void => {
      if (disposed) return; disposed = true;
      fire('dispose-registration', token);
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
    panels: { register(def: Record<string, any>) { return registration('panels', { id: def.id, title: def.title, icon: def.icon }, [], def); }, list: () => list('panels').map(item => item.id), open: (id: string, options?: unknown) => fire('invoke', 'panels', 'open', [id, options]), close: (id: string) => fire('invoke', 'panels', 'close', [id]), refresh: (id: string) => fire('invoke', 'panels', 'refresh', [id]), isOpen: () => false },
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
  Object.defineProperty(api, '__sandbox', { value: { update(next: SandboxMirror) { mirror = next; }, ready: () => Promise.all([...registrations]), dispose() { for (const fn of disposers.reverse()) fn(); }, mountPanel(_viewPort: MessagePort) { throw new Error('Panel views arrive with the next update'); } } });
  return api as PowermoveAPI;
}
