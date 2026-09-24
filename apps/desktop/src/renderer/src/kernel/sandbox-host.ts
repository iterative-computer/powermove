import type { ExtensionRecord } from '../../../shared/extensions';
import { createRpc, createRpcBudget, rpcTransfers, type Rpc } from '../../../shared/sandbox-rpc';
import { panelInfo, type SandboxInit, type SandboxKey, type SandboxMirror, type SandboxViewInit } from '../../sandbox/shim-api';
import type { Disposable } from './api';
import { createExtensionAPI, type ExtensionHandle, type HostDeps } from './host';
import type { Kernel } from './registries';
import { themeScheme, themeTokens } from './theme-apply';
import { mountSandboxView, type ViewHost, type ViewLink } from './sandbox-view';
import { chordOfEvent } from './keychord';
import { menuEntriesSchema, paletteEntriesSchema, parseHostEvent, parseInvoke, parseRegistration } from './sandbox-schemas';

const MIRROR_EVENTS = new Set(['project:changed', 'selection', 'time', 'transport']);
export interface SandboxRuntime { handle: ExtensionHandle; dispose(): void }
const SAFE_INVOKE: Record<string, Set<string>> = {
  commands: new Set(['run']), project: new Set(['apply', 'select', 'setTime', 'play', 'pause', 'undo', 'redo', 'snapshot']),
  transport: new Set(['step']), assets: new Set(['pick', 'import', 'get', 'readText']),
  storage: new Set(['get', 'set', 'delete']), ui: new Set(['toast', 'confirm', 'icon']),
  panels: new Set(['open', 'close', 'refresh']), keybindings: new Set(['unbind']),
  theme: new Set(['activate', 'setScheme']), palette: new Set(['open']),
  media: new Set(['getImportDefaults']), events: new Set(['emit']),
  extensions: new Set(['setUp'])
};
const HOST_EVENTS = new Set(['project:changed', 'selection', 'time', 'transport', 'theme', 'extensions:changed']);
const ownId = (extension: string, id: string): boolean => id.startsWith(`${extension}.`) || id.startsWith(`${extension}-`);
function denied(message: string, code = 'permission_denied'): never {
  const error = new Error(message) as Error & { code: string };
  error.name = 'PermissionError'; error.code = code; throw error;
}
type PlainMode = 'project' | 'asset-map' | 'asset' | 'secrets';
const MIRROR_LIMIT = 8 * 1024 * 1024;
class MirrorTooLargeError extends Error {}
function chargeMirror(budget: { bytes: number } | undefined, value: string): void {
  if (!budget) return;
  if (value.length > MIRROR_LIMIT - budget.bytes) throw new MirrorTooLargeError();
  budget.bytes += new TextEncoder().encode(value).byteLength;
  if (budget.bytes > MIRROR_LIMIT) throw new MirrorTooLargeError();
}
function plain(value: unknown, seen = new WeakMap<object, unknown>(), mode?: PlainMode, budget?: { bytes: number }): unknown {
  if (value === null || typeof value !== 'object') {
    if (typeof value === 'string') chargeMirror(budget, value);
    return typeof value === 'function' ? undefined : value;
  }
  if (seen.has(value)) return seen.get(value);
  if (value instanceof Date) return value.toISOString();
  if (value instanceof Map) { const entries = [...value].map(([key, item]) => [plain(key, seen, mode, budget), plain(item, seen, mode, budget)]); return entries; }
  if (value instanceof Set) return [...value].map(item => plain(item, seen, mode, budget));
  const target: Record<string, unknown> | unknown[] = Array.isArray(value) ? [] : {};
  seen.set(value, target);
  for (const [key, item] of Object.entries(value)) {
    if (typeof item === 'function' || mode === 'asset' && /blob|source/i.test(key) || mode === 'secrets' && /token|secret|password|key$/i.test(key)) continue;
    chargeMirror(budget, key);
    const nextMode = mode === 'project' ? key === 'assets' ? 'asset-map' : key === 'library' || key === 'notes' ? 'secrets' : undefined
      : mode === 'asset-map' ? 'asset' : mode;
    (target as Record<string, unknown>)[key] = plain(item, seen, nextMode, budget);
  }
  return target;
}
export function projectMirror(api: ExtensionHandle['api']): SandboxMirror {
  const start = performance.now();
  const revision = api.project.revision();
  let project: unknown;
  const budget = { bytes: 0 };
  try { project = plain(api.project.get(), new WeakMap(), 'project', budget); }
  catch (error) { if (!(error instanceof MirrorTooLargeError)) throw error; project = { tooLarge: true, revision }; }
  let selection: unknown;
  try { selection = plain(api.project.selection(), new WeakMap(), undefined, budget); }
  catch (error) { if (!(error instanceof MirrorTooLargeError)) throw error; selection = null; project = { tooLarge: true, revision }; }
  const mirror: SandboxMirror = { project, revision, selection, time: api.project.time(), playing: api.project.playing() };
  if (new TextEncoder().encode(JSON.stringify(mirror)).byteLength > MIRROR_LIMIT) {
    mirror.project = { tooLarge: true, revision };
    if (new TextEncoder().encode(JSON.stringify(mirror)).byteLength > MIRROR_LIMIT) mirror.selection = null;
  }
  if (import.meta.env.DEV && performance.now() - start > 4 && Date.now() - lastCloneLog > 1000) {
    lastCloneLog = Date.now(); console.debug(`[sandbox] mirror clone: ${(performance.now() - start).toFixed(1)} ms`);
  }
  return mirror;
}
let lastCloneLog = 0;
function cached<T>(rpc: Rpc, id: number, fallback: T, map: (value: unknown) => T = value => value as T): (...args: unknown[]) => T {
  let last = fallback;
  return (...args) => { void rpc.invokeHandle(id, ...args).then(value => { last = map(value); }).catch(() => {}); return last; };
}
function themeSnapshot(kernel: Kernel): SandboxInit['theme'] {
  const definition = kernel.themes.get(kernel.theme.activeId);
  const scheme = themeScheme(definition, kernel.theme.scheme);
  return { scheme, tokens: themeTokens(definition, scheme) };
}
/* Views sit inside the app's own panels, so they take the theme as the host
   document shows it: the kernel theme plus the workspace's overrides, both
   written as inline custom properties on <html>. */
function viewTheme(kernel: Kernel): SandboxInit['theme'] {
  const base = themeSnapshot(kernel);
  const root = document.documentElement;
  const tokens: Record<string, string> = { ...base.tokens };
  for (let index = 0; index < root.style.length; index++) {
    const key = root.style.item(index);
    if (key.startsWith('--')) tokens[key] = root.style.getPropertyValue(key).trim();
  }
  const attribute = root.dataset.theme;
  return { scheme: attribute === 'dark' || attribute === 'light' ? attribute : base.scheme, tokens };
}
function keyTable(kernel: Kernel, id: string): SandboxKey[] {
  return kernel.listBindings().filter(binding => binding.ownerId === id && ownId(id, binding.command) && kernel.commands.topEntry(binding.command)?.ownerId === id)
    .map(({ chord, inFields, repeat, looseModifiers }) => ({ chord, inFields, repeat, looseModifiers: looseModifiers === true }));
}
function sandboxTheme(value: Record<string, unknown>): Record<string, unknown> {
  if (value.css) {
    const css = String(value.css);
    const rules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)];
    if (rules.map(match => match[0]).join('').replace(/\s/g, '') !== css.replace(/\s/g, '') ||
      rules.some(match => !/^(?:\s*(?::root|html|body)\s*,?)+$/.test(match[1] ?? '') ||
        (match[2] ?? '').split(';').filter(Boolean).some(declaration => !/^\s*--[\w-]+\s*:\s*[^;{}]+\s*$/.test(declaration) || /url\s*\(|@|\\/.test(declaration)))) {
      throw new Error('Sandbox theme CSS may only set custom properties on :root, html, or body');
    }
  }
  return { ...value, rootAttributes: undefined };
}

/** Starts a store extension behind an opaque-origin script-only iframe. */
export async function createSandboxRuntime(kernel: Kernel, record: ExtensionRecord, deps: HostDeps, vars: Record<string, string> = {}, test?: {
  frame?: HTMLIFrameElement;
  onPostInit?(port: MessagePort, init: SandboxInit): void;
  /** Deliver a view's `init` without loading a document (views get no `src`). */
  onViewInit?(frame: HTMLIFrameElement, message: SandboxViewInit & { t: 'init' }, ports: MessagePort[]): void;
}): Promise<SandboxRuntime> {
  const manifest = record.manifest;
  if (!manifest) throw new Error('Sandbox manifest missing');
  const host = createExtensionAPI(kernel, record, deps, vars);
  const frame = test?.frame ?? document.createElement('iframe');
  frame.hidden = true;
  frame.setAttribute('sandbox', 'allow-scripts');
  frame.setAttribute('aria-hidden', 'true');
  const base = location.protocol === 'app:' ? 'app://powermove' : location.origin;
  const perms = (manifest.permissions ?? []).join(',');
  if (!test?.frame) frame.src = `${base}/host/ext-sandbox.html?id=${encodeURIComponent(record.id)}&perms=${encodeURIComponent(perms)}`;
  const registrations = new Map<string, Disposable>();
  let registrationCount = 0;
  const channel = new MessageChannel();
  const budget = createRpcBudget();
  let activated: () => void = () => {};
  let rejected: (error: Error) => void = () => {};
  const ready = new Promise<void>((resolve, reject) => { activated = resolve; rejected = reject; });
  void ready.catch(() => {}); // a load failure can settle before activation is awaited
  const permissions = manifest.permissions ?? [];
  const violations = new Set<string>();
  const persistedStorage = (deps.pm as { store?: { get?: (key: string, fallback: unknown) => unknown } }).store?.get?.(`ext.${record.id}`, {});
  const storageValues = new Map<string, unknown>(persistedStorage && typeof persistedStorage === 'object' && !Array.isArray(persistedStorage)
    ? Object.entries(persistedStorage) : []);
  const invoke = (namespace: string, method: string, args: unknown): unknown => {
    if (!SAFE_INVOKE[namespace]?.has(method)) throw new Error(`Sandbox method unavailable: ${namespace}.${method}`);
    const parsed = parseInvoke(namespace, method, args);
    if (!permissions.includes('project:write') && (
      namespace === 'project' && method !== 'snapshot' || namespace === 'transport' ||
      namespace === 'commands' && !ownId(record.id, String(parsed[0]))))
      denied(`${namespace}.${method} requires project:write permission; commands.run may call only your own registered commands without it`, 'project:write');
    if (namespace === 'commands' && ownId(record.id, String(parsed[0])) && kernel.commands.topEntry(String(parsed[0]))?.ownerId !== record.id)
      denied('commands.run may call only your own registered commands', 'permission_denied');
    if (namespace === 'extensions' && parsed[0] !== record.id) denied('extensions.setUp accepts only the calling extension id');
    if (namespace === 'events' && !String(parsed[0]).startsWith(`${record.id}:`)) denied('events.emit requires your own namespace');
    if (namespace === 'keybindings' && parsed[1] === true) denied('keybindings.unbind cannot affect other owners');
    if (namespace === 'theme' && method === 'activate' && !ownId(record.id, String(parsed[0]))) denied('theme.activate accepts only your themes');
    if (namespace === 'assets' && method !== 'get' && !permissions.includes('assets')) {
      const error = new Error(`assets.${method} requires assets permission`); error.name = 'PermissionError'; throw error;
    }
    if (namespace === 'storage') {
      const key = String(parsed[0]);
      if (key.length > 128) denied('storage key exceeds 128 characters', 'resource_limit');
      if (method === 'set') {
        const next = new Map(storageValues); next.set(key, parsed[1]);
        if (new TextEncoder().encode(JSON.stringify(Object.fromEntries(next))).byteLength > 256 * 1024) denied('storage exceeds 256 KiB', 'resource_limit');
        storageValues.set(key, parsed[1]);
      }
      if (method === 'delete') storageValues.delete(key);
    }
    const receiver = (host.api as unknown as Record<string, Record<string, (...a: unknown[]) => unknown>>)[namespace];
    return receiver?.[method]?.(...parsed);
  };
  /* Handlers every extension document gets: the runtime iframe and each
     panel view. Only the runtime may register contributions; a view may only
     subscribe to events (see shim-api.ts, view mode). */
  let logWindow = 0, logCount = 0;
  const shared = (link: { rpc: Rpc; registrations: Map<string, Disposable> }, runtime: boolean): Record<string, (...args: any[]) => unknown> => ({
    register(kind: string, token: string, input: unknown) {
      const { registrations } = link;
      if (typeof token !== 'string' || !token || token.length > 128) denied('Invalid sandbox registration token', 'resource_limit');
      if (registrations.has(token)) throw new Error('Duplicate sandbox registration');
      if (!runtime && kind !== 'events') throw new Error(`Panel views cannot register ${kind}`);
      if (registrationCount >= 200) denied('Sandbox registration limit is 200', 'resource_limit');
      const value = parseRegistration(kind, input);
      const id = value.id;
      if (typeof id === 'string') {
        if (!ownId(record.id, id)) denied(`Registration id must start with ${record.id}. or ${record.id}-`, 'id_collision');
        const registry = ({ commands: kernel.commands, effects: kernel.effects, transitions: kernel.transitions,
          layers: kernel.layerTypes, theme: kernel.themes, status: kernel.status, panels: kernel.panels } as Record<string, { topEntry(id: string): { ownerId: string } | undefined }>)[kind];
        const owner = registry?.topEntry(id)?.ownerId;
        if (owner && owner !== record.id) denied(`Registration id ${id} belongs to ${owner}`, 'id_collision');
      }
      if (kind === 'keybindings' && (!ownId(record.id, String(value.command)) || kernel.commands.topEntry(String(value.command))?.ownerId !== record.id))
        denied('Keybindings may reference only your registered commands', 'permission_denied');
      if (kind === 'events' && !String(value.event).startsWith(`${record.id}:`) && !HOST_EVENTS.has(String(value.event)))
        denied('Event subscriptions require your namespace or a read-only host event');
      const rpc = link.rpc;
      let item: Disposable;
      switch (kind) {
        case 'effects': item = host.api.effects.register(value as unknown as Parameters<typeof host.api.effects.register>[0]); break;
        case 'transitions': item = host.api.transitions.register(value as unknown as Parameters<typeof host.api.transitions.register>[0]); break;
        case 'layers': item = host.api.layers.register(value as unknown as Parameters<typeof host.api.layers.register>[0]); break;
        case 'theme': item = host.api.theme.register(sandboxTheme(value) as unknown as Parameters<typeof host.api.theme.register>[0]); break;
        case 'keybindings': item = host.api.keybindings.bind(value as unknown as Parameters<typeof host.api.keybindings.bind>[0]); break;
        case 'media-defaults': item = host.api.media.registerImportDefaults(value as unknown as Parameters<typeof host.api.media.registerImportDefaults>[0]); break;
        case 'commands': item = host.api.commands.register({ ...value, id: String(value.id), label: String(value.label), run: (...args: unknown[]) => rpc.invokeHandle(Number(value.run), ...args), ...(value.when ? { when: cached(rpc, Number(value.when), true) } : {}) }); break;
        case 'status': item = host.api.status.register({ ...value, id: String(value.id), text: cached(rpc, Number(value.text), null), ...(value.onClick ? { onClick: () => void rpc.invokeHandle(Number(value.onClick)) } : {}) }); break;
        case 'palette': item = host.api.palette.registerProvider(cached(rpc, Number(value.provider), [], result => paletteEntriesSchema.parse(result).map(entry => {
          if (!ownId(record.id, entry.id) || kernel.commands.topEntry(entry.id)?.ownerId && kernel.commands.topEntry(entry.id)?.ownerId !== record.id) denied('Palette entry id collides with another owner', 'id_collision');
          return { ...entry, run: () => rpc.invokeHandle(entry.run) };
        }))); break;
        case 'menus': item = host.api.menus.contribute(value.location as Parameters<typeof host.api.menus.contribute>[0], cached(rpc, Number(value.items), [], result => menuEntriesSchema.parse(result).map(entry => {
          if (typeof entry === 'string' || !('run' in entry) || !entry.run) return entry;
          const run = entry.run;
          return { ...entry, run: () => rpc.invokeHandle(run) };
        })) as Parameters<typeof host.api.menus.contribute>[1]); break;
        case 'events': {
          /* A mirror event reaches the sandbox after the mirror it describes:
             flush a pending mirror push first (same port, so ordered). */
          const event = String(value.event);
          const mirrored = MIRROR_EVENTS.has(event);
          const sourceEvent = event === 'theme' ? 'theme:changed' : event;
          item = host.api.events.on(sourceEvent as Parameters<typeof host.api.events.on>[0], (payload: unknown) => {
            let forwarded: unknown;
            try { forwarded = event.startsWith(`${record.id}:`) ? payload : parseHostEvent(event, payload); }
            catch { return; }
            if (mirrored) flushMirror();
            void rpc.invokeHandle(Number(value.fn), plain(forwarded)).catch(() => {});
          });
          break;
        }
        case 'panels': {
          const info = panelInfo(value);
          item = host.registerFramePanel(info, { mount: (body, inst) => mountSandboxView(views, info, body, inst) });
          break;
        }
        default: throw new Error(`Unknown sandbox registration: ${kind}`);
      }
      registrationCount += 1;
      let released = false;
      registrations.set(token, { dispose() { if (released) return; released = true; registrationCount -= 1; item.dispose(); } });
    },
    'dispose-registration'(token: string) { link.registrations.get(token)?.dispose(); link.registrations.delete(token); },
    invoke,
    'extensions-list'() { return host.api.extensions.list().map(({ dir: _dir, ...rest }) => rest); },
    log(level: unknown, message: unknown, data: unknown) { const now = Date.now(); if (now - logWindow >= 1000) { logWindow = now; logCount = 0; } if (++logCount > 50) return; if (level === 'info' || level === 'warn' || level === 'error') host.api.log(level, String(message).slice(0, 4096), ...(Array.isArray(data) ? data : [])); },
    'runtime-error': (error: { message: string }) => deps.reportRuntimeError(record.id, new Error(error?.message)),
    /* Each open panel replays activate in its own document, so one blocked
       request can surface once per document. Count it once per extension
       session, or opening panels alone would trip the auto-disable rule. */
    'csp-violation': (event: { directive: string; blockedURI: string }) => {
      const key = `${event?.directive} ${event?.blockedURI}`;
      if (violations.has(key)) return;
      violations.add(key);
      deps.reportRuntimeError(record.id, new Error(`CSP blocked ${event?.blockedURI} (${event?.directive})`));
    }
  });
  const runtimeLink = { registrations } as { rpc: Rpc; registrations: Map<string, Disposable> };
  const rpc = createRpc(channel.port1, {
    ...shared(runtimeLink, true),
    activated: () => activated(),
    'activation-error': (error: { message: string }) => rejected(new Error(error.message))
  }, 10_000, { budget, onSustainedLimit: () => deps.reportRuntimeError(record.id, new Error('Sandbox RPC rate exceeded 200 messages per second for 3 seconds')) });
  runtimeLink.rpc = rpc;
  const snapshot = (): SandboxInit => ({
    id: record.id, apiVersion: manifest.apiVersion ?? 1, manifest, vars,
    theme: themeSnapshot(kernel), activeTheme: kernel.theme.activeId,
    project: projectMirror(host.api), bundleUrl: record.bundleUrl ?? `${base}/ext/${encodeURIComponent(record.id)}/bundle.js`,
    catalog: {
      effects: plain(kernel.effects.list()) as Array<Record<string, unknown>>,
      transitions: plain(kernel.transitions.list()) as Array<Record<string, unknown>>,
      layers: plain(kernel.layerTypes.list()) as Array<Record<string, unknown>>,
      theme: plain(kernel.themes.list()) as Array<Record<string, unknown>>,
      keybindings: plain(kernel.listBindings()) as Array<Record<string, unknown>>,
      commands: kernel.commands.list().map(({ run: _run, when: _when, ...entry }) => entry),
      panels: kernel.panels.list().map(({ id, title, icon }) => ({ id, title, icon })),
      status: kernel.status.list().map(({ id, title, side }) => ({ id, title, side }))
    }
  });
  const links = new Set<ViewLink>();
  const broadcast = (method: string, value: unknown): void => {
    for (const link of links) { try { link.rpc.notify(method, value); } catch { /* view closing */ } }
  };
  const views: ViewHost = {
    src: panelId => test?.onViewInit ? null : `${base}/host/ext-sandbox.html?id=${encodeURIComponent(record.id)}&view=${encodeURIComponent(panelId)}&perms=${encodeURIComponent(perms)}`,
    snapshot,
    theme: () => viewTheme(kernel),
    keys: () => keyTable(kernel, record.id),
    budget,
    forwardKey: payload => {
      const chord = chordOfEvent(payload);
      if (!chord) return;
      if (chord === 'escape') { (deps.pm as { closeMenus?: () => void }).closeMenus?.(); return; }
      if (chord === 'tab' || chord === 'shift+tab') return;
      for (const binding of kernel.bindingsFor(chord)) {
        if (binding.ownerId !== record.id || !ownId(record.id, binding.command) || kernel.commands.topEntry(binding.command)?.ownerId !== record.id) continue;
        if (payload.field && !binding.inFields || payload.repeat && !binding.repeat) continue;
        void Promise.resolve(host.api.commands.run(binding.command, ...(binding.args ?? [])))
          .catch(error => deps.reportRuntimeError(record.id, error));
        break;
      }
    },
    outsideClick: () => (deps.pm as { closeMenus?: () => void }).closeMenus?.(),
    links,
    connectRuntime: (panelId, token, port) => rpc.notify('mountPanel', panelId, token, port, rpcTransfers(port)),
    disconnectRuntime: token => { try { rpc.notify('unmountPanel', token); } catch { /* runtime already gone */ } },
    handlers: link => shared(link, false),
    report: error => deps.reportRuntimeError(record.id, error),
    ...(test?.onViewInit ? { post: test.onViewInit } : {})
  };
  /* Keys and theme follow the host while views are open. */
  let keysQueued = false;
  const keysOff = kernel.keybindings.onChange(() => {
    if (keysQueued || !links.size) return; keysQueued = true;
    queueMicrotask(() => { keysQueued = false; broadcast('keys', keyTable(kernel, record.id)); });
  });
  let themeQueued = false;
  const themeWatch = typeof MutationObserver === 'function' ? new MutationObserver(() => {
    if (themeQueued || !links.size) return; themeQueued = true;
    requestAnimationFrame(() => { themeQueued = false; broadcast('theme', viewTheme(kernel)); });
  }) : null;
  themeWatch?.observe(document.documentElement, { attributes: true, attributeFilter: ['style', 'data-theme'] });
  let disposed = false;
  const dispose = (): void => {
    if (disposed) return; disposed = true;
    keysOff.dispose(); themeWatch?.disconnect();
    // Registrations first: panel views tell the runtime to close their ports.
    for (const item of registrations.values()) item.dispose(); registrations.clear();
    try { rpc.notify('dispose'); } catch { /* already closed */ }
    rpc.close();
    host.disposeAll(); frame.remove();
  };
  /* One mirror push per frame to the runtime and every open view. */
  let dirty = false;
  let scheduled = false;
  function flushMirror(): void {
    if (!dirty || disposed) return; dirty = false;
    try {
      const mirror = projectMirror(host.api);
      rpc.notify('mirror', mirror);
      broadcast('mirror', mirror);
    } catch (error) { deps.reportRuntimeError(record.id, error); }
  }
  const push = (): void => {
    dirty = true;
    if (scheduled) return; scheduled = true;
    requestAnimationFrame(() => { scheduled = false; flushMirror(); });
  };
  for (const event of MIRROR_EVENTS) host.api.events.on(event as 'project:changed', push);
  host.api.events.on('theme:changed', () => { try { rpc.notify('theme', themeSnapshot(kernel)); } catch { /* disposed */ } });
  try {
    host.setActivating(true);
    const loaded = new Promise<void>((resolve, reject) => { frame.addEventListener('load', () => resolve(), { once: true }); frame.addEventListener('error', () => reject(new Error('Sandbox document failed to load')), { once: true }); });
    const timeout = setTimeout(() => { const error = new Error('Sandbox document or activation timed out'); rejected(error); frame.dispatchEvent(new Event('error')); }, 9_000);
    document.body.append(frame);
    try {
      await loaded;
      const init = snapshot();
      if (test?.onPostInit) test.onPostInit(channel.port2, init);
      else frame.contentWindow?.postMessage({ t: 'init', ...init }, '*', [channel.port2]);
      await ready;
    } finally { clearTimeout(timeout); }
    host.setActivating(false);
    return { handle: host, dispose };
  } catch (error) { host.setActivating(false); dispose(); throw error; }
}
