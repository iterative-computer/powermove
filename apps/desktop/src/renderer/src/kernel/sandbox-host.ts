import type { ExtensionRecord } from '../../../shared/extensions';
import { createRpc, rpcTransfers, type Rpc } from '../../../shared/sandbox-rpc';
import { panelInfo, type SandboxInit, type SandboxKey, type SandboxMirror, type SandboxViewInit } from '../../sandbox/shim-api';
import type { Disposable } from './api';
import { createExtensionAPI, type ExtensionHandle, type HostDeps } from './host';
import type { Kernel } from './registries';
import { themeScheme, themeTokens } from './theme-apply';
import { mountSandboxView, type ViewHost, type ViewLink } from './sandbox-view';

const MIRROR_EVENTS = new Set(['project:changed', 'selection', 'time', 'transport']);
export interface SandboxRuntime { handle: ExtensionHandle; dispose(): void }
const SAFE_INVOKE: Record<string, Set<string>> = {
  commands: new Set(['run']), project: new Set(['apply', 'select', 'setTime', 'play', 'pause', 'undo', 'redo', 'snapshot']),
  transport: new Set(['step']), assets: new Set(['pick', 'import', 'get', 'readText']),
  storage: new Set(['get', 'set', 'delete']), ui: new Set(['toast', 'confirm', 'icon']),
  panels: new Set(['open', 'close', 'refresh']), keybindings: new Set(['unbind']),
  theme: new Set(['activate', 'setScheme']), palette: new Set(['open']),
  media: new Set(['getImportDefaults']), events: new Set(['emit']),
  extensions: new Set(['setEnabled', 'remove', 'reload', 'reveal'])
};
function plain(value: unknown, seen = new WeakMap<object, unknown>()): unknown {
  if (value === null || typeof value !== 'object') return typeof value === 'function' ? undefined : value;
  if (seen.has(value)) return seen.get(value);
  if (value instanceof Date) return value.toISOString();
  if (value instanceof Map) { const entries = [...value].map(([key, item]) => [plain(key, seen), plain(item, seen)]); return entries; }
  if (value instanceof Set) return [...value].map(item => plain(item, seen));
  const target: any = Array.isArray(value) ? [] : {};
  seen.set(value, target);
  for (const [key, item] of Object.entries(value)) if (typeof item !== 'function') target[key] = plain(item, seen);
  return target;
}
export function projectMirror(api: ExtensionHandle['api']): SandboxMirror {
  const start = performance.now();
  const mirror = { project: plain(api.project.get()), revision: api.project.revision(), selection: plain(api.project.selection()), time: api.project.time(), playing: api.project.playing() } as SandboxMirror;
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
function keyTable(kernel: Kernel): SandboxKey[] {
  return kernel.listBindings().map(({ chord, inFields, repeat, looseModifiers }) => ({ chord, inFields, repeat, looseModifiers: looseModifiers === true }));
}
function sandboxTheme(value: Record<string, any>): Record<string, any> {
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
  const host = createExtensionAPI(kernel, record, deps, vars);
  const frame = test?.frame ?? document.createElement('iframe');
  frame.hidden = true;
  frame.setAttribute('sandbox', 'allow-scripts');
  frame.setAttribute('aria-hidden', 'true');
  const base = location.protocol === 'app:' ? 'app://powermove' : location.origin;
  const perms = (record.manifest?.permissions ?? []).join(',');
  if (!test?.frame) frame.src = `${base}/host/ext-sandbox.html?id=${encodeURIComponent(record.id)}&perms=${encodeURIComponent(perms)}`;
  const registrations = new Map<string, Disposable>();
  const channel = new MessageChannel();
  let activated!: () => void;
  let rejected!: (error: Error) => void;
  const ready = new Promise<void>((resolve, reject) => { activated = resolve; rejected = reject; });
  void ready.catch(() => {}); // a load failure can settle before activation is awaited
  const permissions = record.manifest?.permissions ?? [];
  const violations = new Set<string>();
  const invoke = (namespace: string, method: string, args: unknown[]): unknown => {
    if (!SAFE_INVOKE[namespace]?.has(method)) throw new Error(`Sandbox method unavailable: ${namespace}.${method}`);
    if (namespace === 'project' && method === 'apply' && !permissions.includes('project:write')) {
      const error = new Error('project.apply requires project:write permission'); error.name = 'PermissionError'; throw error;
    }
    if (namespace === 'assets' && method !== 'get' && !permissions.includes('assets')) {
      const error = new Error(`assets.${method} requires assets permission`); error.name = 'PermissionError'; throw error;
    }
    const receiver = (host.api as unknown as Record<string, Record<string, (...a: any[]) => unknown>>)[namespace];
    return receiver?.[method]?.(...(Array.isArray(args) ? args : []));
  };
  /* Handlers every extension document gets: the runtime iframe and each
     panel view. Only the runtime may register contributions; a view may only
     subscribe to events (see shim-api.ts, view mode). */
  const shared = (link: { rpc: Rpc; registrations: Map<string, Disposable> }, runtime: boolean): Record<string, (...args: any[]) => unknown> => ({
    register(kind: string, token: string, value: Record<string, any>) {
      const { registrations } = link;
      if (registrations.has(token)) throw new Error('Duplicate sandbox registration');
      if (!runtime && kind !== 'events') throw new Error(`Panel views cannot register ${kind}`);
      const rpc = link.rpc;
      let item: Disposable;
      switch (kind) {
        case 'effects': item = host.api.effects.register(value as any); break;
        case 'transitions': item = host.api.transitions.register(value as any); break;
        case 'layers': item = host.api.layers.register(value as any); break;
        case 'theme': item = host.api.theme.register(sandboxTheme(value) as any); break;
        case 'keybindings': item = host.api.keybindings.bind(value as any); break;
        case 'media-defaults': item = host.api.media.registerImportDefaults(value as any); break;
        case 'commands': item = host.api.commands.register({ ...value, run: (...args: unknown[]) => rpc.invokeHandle(value.run, ...args), ...(value.when ? { when: cached(rpc, value.when, true) } : {}) } as any); break;
        case 'status': item = host.api.status.register({ ...value, text: cached(rpc, value.text, null), ...(value.onClick ? { onClick: () => void rpc.invokeHandle(value.onClick) } : {}) } as any); break;
        case 'palette': item = host.api.palette.registerProvider(cached(rpc, value.provider, [], result => (result as Array<Record<string, any>>).map(entry => ({ ...entry, run: () => rpc.invokeHandle(entry.run) }))) as any); break;
        case 'menus': item = host.api.menus.contribute(value.location, cached(rpc, value.items, [], result => (result as Array<string | Record<string, any>>).map(entry => typeof entry === 'string' || !entry.run ? entry : { ...entry, run: () => rpc.invokeHandle(entry.run) })) as any); break;
        case 'events': {
          /* A mirror event reaches the sandbox after the mirror it describes:
             flush a pending mirror push first (same port, so ordered). */
          const mirrored = MIRROR_EVENTS.has(value.event);
          item = host.api.events.on(value.event, ((payload: unknown) => { if (mirrored) flushMirror(); void rpc.invokeHandle(value.fn, plain(payload)).catch(() => {}); }) as any);
          break;
        }
        case 'panels': {
          const info = panelInfo(value);
          item = host.registerFramePanel(info, { mount: (body, inst) => mountSandboxView(views, info, body, inst) });
          break;
        }
        default: throw new Error(`Unknown sandbox registration: ${kind}`);
      }
      registrations.set(token, item);
    },
    'dispose-registration'(token: string) { link.registrations.get(token)?.dispose(); link.registrations.delete(token); },
    invoke,
    'extensions-list'() { return host.api.extensions.list().map(({ dir: _dir, ...rest }) => rest); },
    log(level: 'info' | 'warn' | 'error', message: string, data: unknown[]) { host.api.log(level, message, ...(Array.isArray(data) ? data : [])); },
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
  });
  runtimeLink.rpc = rpc;
  const snapshot = (): SandboxInit => ({
    id: record.id, apiVersion: record.manifest?.apiVersion ?? 1, manifest: record.manifest!, vars,
    theme: themeSnapshot(kernel), activeTheme: kernel.theme.activeId,
    project: projectMirror(host.api), bundleUrl: record.bundleUrl ?? `${base}/ext/${encodeURIComponent(record.id)}/bundle.js`,
    catalog: {
      effects: plain(kernel.effects.list()) as Array<Record<string, any>>,
      transitions: plain(kernel.transitions.list()) as Array<Record<string, any>>,
      layers: plain(kernel.layerTypes.list()) as Array<Record<string, any>>,
      theme: plain(kernel.themes.list()) as Array<Record<string, any>>,
      keybindings: plain(kernel.listBindings()) as Array<Record<string, any>>,
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
    keys: () => keyTable(kernel),
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
    queueMicrotask(() => { keysQueued = false; broadcast('keys', keyTable(kernel)); });
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
  for (const event of MIRROR_EVENTS) host.api.events.on(event as 'project:changed', push as any);
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
