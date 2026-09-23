import type { ExtensionRecord } from '../../../shared/extensions';
import { createRpc, type Rpc } from '../../../shared/sandbox-rpc';
import type { SandboxInit, SandboxMirror } from '../../sandbox/shim-api';
import type { Disposable } from './api';
import { createExtensionAPI, type ExtensionHandle, type HostDeps } from './host';
import type { Kernel } from './registries';
import { themeScheme, themeTokens } from './theme-apply';

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
  const rpc = createRpc(channel.port1, {
    register(kind: string, token: string, value: Record<string, any>) {
      if (registrations.has(token)) throw new Error('Duplicate sandbox registration');
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
        case 'events': item = host.api.events.on(value.event, ((payload: unknown) => void rpc.invokeHandle(value.fn, plain(payload))) as any); break;
        case 'panels': item = host.api.panels.register({ id: value.id, title: value.title, icon: value.icon, build: body => { body.textContent = 'Panel UI arrives with the next update'; } }); break;
        default: throw new Error(`Unknown sandbox registration: ${kind}`);
      }
      registrations.set(token, item);
    },
    'dispose-registration'(token: string) { registrations.get(token)?.dispose(); registrations.delete(token); },
    mountPanel(_viewPort: MessagePort) { throw new Error('Panel UI arrives with the next update'); },
    invoke(namespace: string, method: string, args: unknown[]) {
      if (!SAFE_INVOKE[namespace]?.has(method)) throw new Error(`Sandbox method unavailable: ${namespace}.${method}`);
      if (namespace === 'project' && method === 'apply' && !(record.manifest?.permissions ?? []).includes('project:write')) {
        const error = new Error('project.apply requires project:write permission'); error.name = 'PermissionError'; throw error;
      }
      if (namespace === 'assets' && method !== 'get' && !(record.manifest?.permissions ?? []).includes('assets')) {
        const error = new Error(`assets.${method} requires assets permission`); error.name = 'PermissionError'; throw error;
      }
      const receiver = (host.api as unknown as Record<string, Record<string, (...a: any[]) => unknown>>)[namespace];
      return receiver?.[method]?.(...args);
    },
    'extensions-list'() { return host.api.extensions.list().map(({ dir: _dir, ...rest }) => rest); },
    log(level: 'info' | 'warn' | 'error', message: string, data: unknown[]) { host.api.log(level, message, ...data); },
    activated: () => activated(),
    'activation-error': (error: { message: string }) => rejected(new Error(error.message)),
    'runtime-error': (error: { message: string }) => deps.reportRuntimeError(record.id, new Error(error.message)),
    'csp-violation': (event: { directive: string; blockedURI: string }) => deps.reportRuntimeError(record.id, new Error(`CSP blocked ${event.blockedURI} (${event.directive})`))
  });
  const dispose = (): void => {
    rpc.notify('dispose'); rpc.close();
    for (const item of registrations.values()) item.dispose(); registrations.clear();
    host.disposeAll(); frame.remove();
  };
  let scheduled = false;
  const push = (): void => {
    if (scheduled) return; scheduled = true;
    requestAnimationFrame(() => { scheduled = false; try { rpc.notify('mirror', projectMirror(host.api)); } catch (error) { deps.reportRuntimeError(record.id, error); } });
  };
  for (const event of ['project:changed', 'selection', 'time', 'transport'] as const) host.api.events.on(event, push as any);
  host.api.events.on('theme:changed', () => rpc.notify('theme', themeSnapshot(kernel)));
  try {
    host.setActivating(true);
    const loaded = new Promise<void>((resolve, reject) => { frame.addEventListener('load', () => resolve(), { once: true }); frame.addEventListener('error', () => reject(new Error('Sandbox document failed to load')), { once: true }); });
    const timeout = setTimeout(() => { const error = new Error('Sandbox document or activation timed out'); rejected(error); frame.dispatchEvent(new Event('error')); }, 9_000);
    document.body.append(frame);
    try {
      await loaded;
    const bundleUrl = record.bundleUrl ?? `${base}/ext/${encodeURIComponent(record.id)}/bundle.js`;
    const init: SandboxInit = {
      id: record.id, apiVersion: record.manifest?.apiVersion ?? 1, manifest: record.manifest!, vars,
      theme: themeSnapshot(kernel), activeTheme: kernel.theme.activeId,
      project: projectMirror(host.api), bundleUrl,
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
    };
    if (test?.onPostInit) test.onPostInit(channel.port2, init);
    else frame.contentWindow?.postMessage({ t: 'init', ...init }, '*', [channel.port2]);
      await ready;
    } finally { clearTimeout(timeout); }
    host.setActivating(false);
    return { handle: host, dispose };
  } catch (error) { host.setActivating(false); dispose(); throw error; }
}
