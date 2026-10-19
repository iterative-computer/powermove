import type { ExtensionRecord } from '../../../shared/extensions';
import { createRpc, createRpcBudget, rpcTransfers, type Rpc } from '../../../shared/sandbox-rpc';
import { panelInfo, type SandboxEvent, type SandboxInit, type SandboxKey, type SandboxSnapshot, type SandboxState, type SandboxViewInit } from '../../sandbox/shim-api';
import type { Disposable, MenuContribution, Selection } from './api';
import { createExtensionAPI, type ExtensionHandle, type HostDeps } from './host';
import { ASYNC_CONTRIBUTOR, type Kernel } from './registries';
import { themeScheme, themeTokens } from './theme-apply';
import { mountSandboxView, type ViewHost, type ViewLink } from './sandbox-view';
import { chordOfEvent, normalizeChord } from './keychord';
import { bridge } from './bridge';
import { plain, projectSnapshots, sandboxStats } from './project-snapshots';
import { sandboxBundleUrl, sandboxDocumentUrl, sandboxOrigin } from '../../../shared/sandbox-origin';
import { watchSandbox } from './sandbox-watchdog';
import { importedFile, menuEntriesSchema, paletteEntriesSchema, parseHostEvent, parseInvoke, parseRegistration } from './sandbox-schemas';
import { sandboxOpenExternal, userActivated } from './sandbox-links';
import { sandboxImportUrl } from './sandbox-import-url';

/** Kernel events that can change a document's SandboxState. */
const STATE_EVENTS = ['project:changed', 'selection', 'time', 'transport'] as const;
/** Kernel events after which other extensions' contributions may differ. */
const CATALOG_EVENTS = ['extensions:changed', 'extension:loaded', 'extension:unloaded'] as const;
export interface SandboxRuntime { handle: ExtensionHandle; dispose(): void }
const SAFE_INVOKE: Record<string, Set<string>> = {
  commands: new Set(['run']), project: new Set(['apply', 'select', 'setTime', 'play', 'pause', 'undo', 'redo', 'snapshot']),
  transport: new Set(['step']), assets: new Set(['pick', 'import', 'get', 'readText', 'importUrl']),
  storage: new Set(['get', 'set', 'delete']), ui: new Set(['toast', 'confirm', 'icon', 'copy', 'openExternal']),
  panels: new Set(['open', 'close', 'refresh', 'isOpen']), keybindings: new Set(['unbind']),
  theme: new Set(['activate']), palette: new Set(['open']),
  media: new Set(['getImportDefaults']), events: new Set(['emit']),
  extensions: new Set(['setUp'])
};
const HOST_EVENTS = new Set(['project:changed', 'selection', 'time', 'transport', 'theme', 'extensions:changed']);
// Own ids are the extension id itself or anything under its `<id>.` namespace.
// Allowing the bare id (it's unique, so it can't impersonate another owner) spares
// the common case of naming your one effect/panel after the extension; the dot is
// still required for suffixes so `foo` can't claim `foo-bar.*`.
export const ownId = (extension: string, id: string): boolean => id === extension || id.startsWith(`${extension}.`);
/** Place a store extension's id in its own namespace (idempotent): a bare `hud`
 * becomes `contour-hud.hud`; anything already in the namespace is left alone. The
 * system does this at the sandbox boundary so authors never prefix, yet the kernel
 * and projects only ever see globally-unique ids. */
export const qualifyId = (extension: string, id: string): string => id && !ownId(extension, id) ? `${extension}.${id}` : id;
/** Distinct CSP violations remembered (and logged) per extension session. */
const MAX_VIOLATION_KEYS = 100;
/* assets.import files skip the RPC byte limit (importedFile); these caps
   hold instead, checked on `file.size` before the host reads a byte. */
const IMPORT_FILE_BYTES = 512 * 1024 * 1024;
const IMPORT_MINUTE_BYTES = 2 * 1024 * 1024 * 1024;
/** How long a person's click or key press stands behind the runtime's ui.openExternal, as long as a transient user activation. */
const PERSON_ACTION_MS = 5_000;
const LEGACY_EDIT_COMMANDS = new Set(['delete', 'duplicate', 'split', 'selectAll', 'deselect', 'groupLayers', 'ungroupLayers', 'nudgeSelection', 'nudgeKeyframes']);
function denied(message: string, code = 'permission_denied'): never {
  const error = new Error(message) as Error & { code: string };
  error.name = 'PermissionError'; error.code = code; throw error;
}
/* One sandbox document (the runtime or a view) as the data plane sees it:
   the state it was last told, the event names it listens to, the
   occurrences waiting for the next flush, and the snapshot generation it was
   last sent in full. */
interface PlaneDoc {
  rpc: Rpc; sent: SandboxState; selection: string;
  interest: Map<string, { count: number; off: Disposable }>;
  pending: SandboxEvent[];
  delivered: number | null;
}
const STATE_KEYS = ['time', 'playing', 'revision', 'generation'] as const;
/** Within one flush `time` and `selection` keep their last value and `project:changed` collapses per kind; everything else stays, in order. */
export function coalesceEvents(events: SandboxEvent[]): SandboxEvent[] {
  if (events.length < 2) return events;
  const seen = new Set<string>();
  const kept: SandboxEvent[] = [];
  for (let index = events.length - 1; index >= 0; index--) {
    const entry = events[index]!;
    const [name, payload] = entry;
    const key = name === 'time' || name === 'selection' ? name : name === 'project:changed' ? `${name}:${String((payload as { kind?: unknown } | null)?.kind)}` : null;
    if (key !== null) { if (seen.has(key)) continue; seen.add(key); }
    kept.push(entry);
  }
  return kept.reverse();
}
export function cached<T>(rpc: Rpc, id: number, fallback: T, map: (value: unknown) => T = value => value as T): (...args: unknown[]) => T {
  let last = fallback;
  let pending = false;
  return (...args) => { if (!pending) { pending = true; void rpc.invokeHandle(id, ...args).then(value => { last = map(value); }).catch(() => {}).finally(() => { pending = false; }); } return last; };
}
/** How long a check waits for a sandboxed command's `when`. */
export const WHEN_DEADLINE_MS = 100;
/**
 * A sandboxed `when`, asked afresh on every check. An answer within
 * `deadlineMs` is the check's; past it the check takes the last answer
 * (`true` before any), and the late one becomes the last when it lands.
 */
export function freshWhen(rpc: Rpc, id: number, deadlineMs = WHEN_DEADLINE_MS): () => Promise<boolean> {
  let last = true, asked = 0, answered = 0;
  return () => new Promise<boolean>((resolve, reject) => {
    const check = ++asked;
    let late = false;
    const timer = setTimeout(() => { late = true; resolve(last); }, deadlineMs);
    rpc.invokeHandle(id).then(value => {
      // An older check answering after a newer one does not overwrite it.
      if (check > answered) { answered = check; last = Boolean(value); }
      clearTimeout(timer); resolve(Boolean(value));
    }, error => { clearTimeout(timer); if (!late) reject(error); });
  });
}
function themeSnapshot(kernel: Kernel): SandboxInit['theme'] {
  const definition = kernel.themes.get(kernel.theme.activeId);
  const scheme = themeScheme(definition, kernel.theme.scheme);
  return { id: kernel.theme.activeId, scheme, tokens: themeTokens(definition, scheme) };
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
  return { id: base.id, scheme: attribute === 'dark' || attribute === 'light' ? attribute : base.scheme, tokens };
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

/**
 * Listens to a sandbox from outside, for the publish-time sandbox check
 * (sandbox-check.ts). With an observer, CSP violations and panel outcomes go
 * to it instead of the loader's error policy.
 */
export interface SandboxObserver {
  /** A trusted-only member was reached (`render.gl`, `ui.menu`, `powermove.resolveContent`). */
  permission?(member: string): void;
  /** apiVersion ≤ 2 code read a property of a method's result that is a Promise here. */
  asyncMisuse?(member: string): void;
  csp?(directive: string, blockedUri: string): void;
  view?(panelId: string, state: 'ready' | 'error', message?: string): void;
}

export interface SandboxRuntimeOptions {
  frame?: HTMLIFrameElement;
  onPostInit?(port: MessagePort, init: SandboxInit): void;
  /** Deliver a view's `init` without loading a document (views get no `src`). */
  onViewInit?(frame: HTMLIFrameElement, message: SandboxViewInit & { t: 'init' }, ports: MessagePort[]): void;
  /** Where registrations land. Defaults to `kernel`; the sandbox check passes a scratch kernel so nothing reaches the app. */
  registry?: Kernel;
  observer?: SandboxObserver;
  /** The whole activation, the liveness probe after a timeout included. Defaults to 10 s, the loader's. */
  timeoutMs?: number;
  /**
   * A runtime outside any document: the document host's contained process
   * (engine/document-host/sandbox.ts). `loaded` settles once it can take
   * `init`; `start` hands it `init` and the kernel's port. It has a process
   * of its own, so it is watched and stopped (`terminate`) like an isolated frame.
   */
  process?: { loaded: Promise<void>; start(port: MessagePort, init: SandboxInit): void; terminate(): Promise<void> };
}

/** Starts a store extension behind an opaque-origin script-only iframe. */
export async function createSandboxRuntime(kernel: Kernel, record: ExtensionRecord, deps: HostDeps, vars: Record<string, string> = {}, test?: SandboxRuntimeOptions): Promise<SandboxRuntime> {
  const observer = test?.observer;
  const reg = test?.registry ?? kernel;
  const manifest = record.manifest;
  if (!manifest) throw new Error('Sandbox manifest missing');
  const host = createExtensionAPI(reg, record, deps, vars);
  const frame = test?.frame ?? document.createElement('iframe');
  frame.hidden = true;
  frame.setAttribute('sandbox', 'allow-scripts allow-forms'); // as a view's (sandbox-view.ts)
  frame.setAttribute('aria-hidden', 'true');
  // Inside Electron the document comes from the extension's own app:// host,
  // so it gets its own process (in development main proxies it from Vite);
  // only the browser host (powermove serve) serves it from its own origin.
  const external = test?.process ?? null;
  // A contained process has no document origin; its ids stay those of the extension's own app:// host.
  const base = sandboxOrigin(record.id, external || location.protocol === 'app:' || navigator.userAgent.includes('Electron') ? 'app://powermove' : location.origin);
  /* Silence is this extension's fault, and stoppable, only when it has a
     process of its own. Under the browser host every sandbox shares one
     origin, so one spinning extension would make its siblings miss pings too,
     and nothing could end it. There no fatal liveness rule runs at all. */
  const isolated = !!external || (base.startsWith('app:') && typeof bridge()?.sandboxTerminate === 'function');
  const perms = (manifest.permissions ?? []).join(',');
  if (!test?.frame && !external) frame.src = sandboxDocumentUrl(base, record.id, perms);
  const registrations = new Map<string, Disposable>();
  const remoteHandles = new Set<number>();
  const claimHandles = (handles: number[]): void => {
    if (new Set([...remoteHandles, ...handles]).size > 2000) denied('Sandbox handle limit is 2000', 'resource_limit');
    for (const handle of handles) remoteHandles.add(handle);
  };
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
  /* assets.import and assets.importUrl share one quota: a download counts at
     its size before the renderer reads it back. */
  const imports: Array<{ at: number; bytes: number }> = [];
  const admitImport = (bytes: number): void => {
    if (bytes > IMPORT_FILE_BYTES) denied('assets.import accepts files up to 512 MiB', 'resource_limit');
    const now = Date.now();
    while (imports.length && now - imports[0]!.at >= 60_000) imports.shift();
    if (imports.reduce((sum, entry) => sum + entry.bytes, bytes) > IMPORT_MINUTE_BYTES) denied('assets.import and assets.importUrl accept up to 2 GiB a minute', 'resource_limit');
    imports.push({ at: now, bytes });
  };
  /* ui.copy: the kernel's manifest record grants it (never the document's
     URL), the host itself sees the calling view focused right after a real
     click or key press, and it writes once a second at most. The runtime has
     no focus to prove, so it never copies. Main checks focus and input again. */
  let copiedAt = -Infinity;
  const copy = async (text: string, view: ViewLink | null): Promise<void> => {
    if (!permissions.includes('clipboard')) denied('ui.copy requires clipboard permission', 'clipboard');
    if (!view?.focused?.()) denied('ui.copy works only from a panel that has focus');
    if (!view.acted?.()) denied('ui.copy works only right after a click or key press in the panel');
    const write = bridge()?.clipboardWriteText;
    if (!write) throw new Error('The clipboard is unavailable');
    const now = Date.now();
    if (now - copiedAt < 1000) denied('ui.copy is limited to once a second', 'resource_limit');
    copiedAt = now;
    await write(text);
  };
  const openExternal = sandboxOpenExternal({ id: record.id, manifest: () => manifest, ui: host.api.ui });
  /* A person's action, as the host saw it, for ui.openExternal's no-sheet
     path: a focused view right after a real click or key press, or, for the
     runtime, a run the host started while the app held that activation (a
     command, a status item's click, a palette, menu or toast item) that is
     still in flight and under 5 s old. The extension's own commands.run is
     never one, nor a key its view forwarded: the host cannot see keys in the
     frame, and the view can send any. */
  let ownRun = false, personRuns = 0, personRunAt = -Infinity;
  const forPerson = <T,>(start: () => Promise<T>): Promise<T> => {
    if (ownRun || !userActivated()) return start();
    personRuns += 1; personRunAt = performance.now();
    let running: Promise<T>;
    try { running = start(); } catch (error) { personRuns -= 1; throw error; }
    return running.finally(() => { personRuns -= 1; });
  };
  const gesture = (view: ViewLink | null): boolean => view
    ? view.acted?.() === true
    : personRuns > 0 && performance.now() - personRunAt < PERSON_ACTION_MS;
  const importUrl = sandboxImportUrl({ manifest: () => manifest, assets: host.api.assets, admit: admitImport });
  const invoke = (namespace: string, method: string, args: unknown, view: ViewLink | null = null): unknown => {
    if (!SAFE_INVOKE[namespace]?.has(method)) throw new Error(`Sandbox method unavailable: ${namespace}.${method}`);
    const parsed = parseInvoke(namespace, method, args);
    if (!permissions.includes('project:write') && (namespace === 'project' && method !== 'snapshot' || namespace === 'transport'))
      denied(`${namespace}.${method} requires project:write permission`, 'project:write');
    if (namespace === 'commands') {
      const raw = String(parsed[0]);
      /* Prefer the extension's own namespaced command (authors call it by the bare
         name); otherwise only approved legacy editing commands may run, by bare id. */
      const qualified = qualifyId(record.id, raw);
      if (reg.commands.topEntry(qualified)?.ownerId === record.id) parsed[0] = qualified;
      else if (!(permissions.includes('project:write') && reg.commands.topEntry(raw)?.ownerId === 'legacy' && LEGACY_EDIT_COMMANDS.has(raw)))
        denied('commands.run may call only your own commands or approved editing commands with project:write', permissions.includes('project:write') ? 'permission_denied' : 'project:write');
    }
    if (namespace === 'extensions' && parsed[0] !== record.id) denied('extensions.setUp accepts only the calling extension id');
    if (namespace === 'events') {
      if (String(parsed[0]).includes(':')) denied('Extension event names cannot contain a namespace separator');
      return host.api.events.emit(`ext:${record.id}:${String(parsed[0])}` as Parameters<typeof host.api.events.emit>[0], parsed[1] as never);
    }
    if (namespace === 'keybindings') { reg.unbind(record.id, String(parsed[0]), false); return; }
    if (namespace === 'ui' && method === 'openExternal') return openExternal(parsed[0], gesture(view));
    if (namespace === 'assets' && method === 'importUrl') return importUrl(parsed[0]);
    if (namespace === 'panels') { parsed[0] = qualifyId(record.id, String(parsed[0])); if (!ownId(record.id, String(parsed[0]))) denied('panels may act only on your own ids'); }
    if (namespace === 'theme' && method === 'activate') { parsed[0] = qualifyId(record.id, String(parsed[0])); if (!ownId(record.id, String(parsed[0]))) denied('theme.activate accepts only your themes'); }
    if (namespace === 'assets' && method !== 'get' && !permissions.includes('assets')) {
      const error = new Error(`assets.${method} requires assets permission`); error.name = 'PermissionError'; throw error;
    }
    if (namespace === 'assets' && method === 'import') admitImport((parsed[0] as File).size);
    if (namespace === 'ui' && method === 'copy') return copy(String(parsed[0]), view);
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
    ownRun = namespace === 'commands';
    try { return receiver?.[method]?.(...parsed); } finally { ownRun = false; }
  };
  /* A toast's `action.run` and `onDismiss` are handles of the document that
     raised it. They are released when the toast closes, however it closes,
     so they never pile up against the handle limits. A toast with buttons,
     or with the extension's own `key`, is keyed by the host and dismissed
     when its document goes (a closed panel, a reload, the extension's
     dispose), so no button is left that can no longer answer. The key is
     namespaced: an extension replaces only its own notices. */
  const toastKeys = new Map<string, { link: object }>();
  let toastSerial = 0;
  const dismissToasts = (link: object | null): void => {
    for (const [key, entry] of [...toastKeys]) {
      if (link !== null && entry.link !== link) continue;
      toastKeys.delete(key);
      (deps.pm as { dismissToast?: (key: string) => void }).dismissToast?.(key);
    }
  };
  const toast = (link: { rpc: Rpc }, live: () => boolean, args: unknown): void => {
    const docRpc = link.rpc;
    const [text, options] = parseInvoke('ui', 'toast', args) as [string, { action?: { label: string; run: number }; onDismiss?: number; key?: unknown } | undefined];
    const handles = [options?.action?.run, options?.onDismiss].filter((id): id is number => typeof id === 'number');
    claimHandles(handles);
    const { action, onDismiss, key: ownKey, ...rest } = options ?? {};
    const key = ownKey !== undefined ? `sandbox:${record.id}:${String(ownKey)}` : handles.length ? `sandbox:${record.id}:#${++toastSerial}` : undefined;
    const entry = { link };
    if (key) toastKeys.set(key, entry);
    let closed = false;
    const onClose = (): void => {
      if (closed) return; closed = true;
      if (key && toastKeys.get(key) === entry) toastKeys.delete(key);
      for (const id of handles) { remoteHandles.delete(id); try { docRpc.release(id); } catch { /* document gone */ } }
    };
    const call = (id: number) => (): Promise<unknown> => docRpc.invokeHandle(id).catch(error => { if (live()) deps.reportRuntimeError(record.id, error); });
    host.api.ui.toast(text, { ...rest, ...(key ? { key } : {}), ...(action ? { action: { label: action.label, run: () => void forPerson(call(action.run)) } } : {}),
      ...(onDismiss !== undefined ? { onDismiss: () => void call(onDismiss)() } : {}), onClose } as Parameters<typeof host.api.ui.toast>[1]);
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
      const handles = ['run', 'when', 'text', 'onClick', 'provider', 'items']
        .map(key => value[key]).filter((handle): handle is number => typeof handle === 'number');
      if (new Set([...remoteHandles, ...handles]).size > 2000) denied('Sandbox handle limit is 2000', 'resource_limit');
      /* System-owned namespacing: a store extension's registration ids — and the
         command a keybinding targets, and palette entry ids below — are placed
         under its own `<id>.` namespace automatically. Authors use any bare id;
         the kernel, editor and projects only ever see globally-unique ids, so
         nothing can shadow a built-in or another extension and no author has to
         prefix anything. The qualified id is still just a string, so the project
         format is unchanged. Built-ins never reach this path. */
      const qualify = (raw: string): string => qualifyId(record.id, raw);
      if (typeof value.id === 'string') value.id = qualify(value.id);
      if (kind === 'keybindings' && typeof value.command === 'string') value.command = qualify(value.command);
      const id = value.id;
      if (typeof id === 'string') {
        /* Always true after qualify(); kept as defense if that ever changes. */
        if (!ownId(record.id, id)) denied(`Registration id must be "${record.id}" or start with "${record.id}."`, 'id_collision');
        const registry = ({ commands: reg.commands, effects: reg.effects, transitions: reg.transitions,
          layers: reg.layerTypes, theme: reg.themes, status: reg.status, panels: reg.panels } as Record<string, { topEntry(id: string): { ownerId: string } | undefined }>)[kind];
        const owner = registry?.topEntry(id)?.ownerId;
        if (owner && owner !== record.id) denied(`Registration id ${id} belongs to ${owner}`, 'id_collision');
      }
      if (kind === 'keybindings' && (!ownId(record.id, String(value.command)) || reg.commands.topEntry(String(value.command))?.ownerId !== record.id))
        denied('Keybindings may reference only your registered commands', 'permission_denied');
      if (kind === 'keybindings' && reg.bindingsFor(normalizeChord(String(value.key))).some(binding => binding.ownerId !== record.id))
        denied('Keybinding chord belongs to another owner', 'id_collision');
      if (kind === 'events' && !HOST_EVENTS.has(String(value.event)) && String(value.event).includes(':'))
        denied('Extension event names cannot contain a namespace separator');
      const rpc = link.rpc;
      let item: Disposable;
      switch (kind) {
        case 'effects': item = host.api.effects.register(value as unknown as Parameters<typeof host.api.effects.register>[0]); break;
        case 'transitions': item = host.api.transitions.register(value as unknown as Parameters<typeof host.api.transitions.register>[0]); break;
        case 'layers': item = host.api.layers.register(value as unknown as Parameters<typeof host.api.layers.register>[0]); break;
        case 'theme': item = host.api.theme.register(sandboxTheme(value) as unknown as Parameters<typeof host.api.theme.register>[0]); break;
        case 'keybindings': item = host.api.keybindings.bind({ ...value, priority: 1000 } as unknown as Parameters<typeof host.api.keybindings.bind>[0]); break;
        case 'media-defaults': item = host.api.media.registerImportDefaults(value as unknown as Parameters<typeof host.api.media.registerImportDefaults>[0]); break;
        case 'commands': item = host.api.commands.register({ ...value, id: String(value.id), label: String(value.label), run: (...args: unknown[]) => forPerson(() => rpc.invokeHandle(Number(value.run), ...args)), ...(value.when ? { when: freshWhen(rpc, Number(value.when)) } : {}) }); break;
        case 'status': item = host.api.status.register({ ...value, id: String(value.id), text: cached(rpc, Number(value.text), null), ...(value.onClick ? { onClick: () => void forPerson(() => rpc.invokeHandle(Number(value.onClick))) } : {}) }); break;
        /* Asked for every query; the palette keeps a reply only while it still
           shows the query that reply answers. */
        case 'palette': item = host.api.palette.registerProvider(query => rpc.invokeHandle(Number(value.provider), query)
          .then(result => paletteEntriesSchema.parse(result).map(entry => {
            claimHandles([entry.run]);
            const entryId = qualify(entry.id);
            if (reg.commands.topEntry(entryId)?.ownerId && reg.commands.topEntry(entryId)?.ownerId !== record.id) denied('Palette entry id collides with another owner', 'id_collision');
            return { ...entry, id: entryId, run: () => forPerson(() => rpc.invokeHandle(entry.run)) };
          }))); break;
        /* Asked afresh on every open, with that open's ctx: the reply to this
           call is the only one these items can come from, so a menu never
           shows (or runs) items built for another target. The kernel bounds
           the wait. */
        case 'menus': item = host.api.menus.contribute(value.location as Parameters<typeof host.api.menus.contribute>[0], Object.assign((ctx: Record<string, unknown>) => rpc.invokeHandle(Number(value.items), ctx)
          .then(result => menuEntriesSchema.parse(result).map(entry => {
            if (typeof entry === 'string' || !('run' in entry) || !entry.run) return entry;
            const run = entry.run;
            claimHandles([run]);
            return { ...entry, run: () => forPerson(() => rpc.invokeHandle(run)) };
          }) as MenuContribution[]), { [ASYNC_CONTRIBUTOR]: true })); break;
        case 'events': {
          const doc = docOf.get(link);
          if (!doc) throw new Error('Sandbox document is not connected');
          item = interest(doc, String(value.event));
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
      claimHandles(handles);
      let released = false;
      registrations.set(token, { dispose() { if (released) return; released = true; registrationCount -= 1; for (const handle of handles) remoteHandles.delete(handle); item.dispose(); } });
    },
    'dispose-registration'(token: string) { link.registrations.get(token)?.dispose(); link.registrations.delete(token); },
    invoke: (namespace: string, method: string, args: unknown) => namespace === 'ui' && method === 'toast'
      ? toast(link, () => !disposed && (runtime || links.has(link as ViewLink)), args) : invoke(namespace, method, args, runtime ? null : link as ViewLink),
    /* Enforced here whatever the shim does: at most one full copy per
       generation for each document. A document asking again for the
       generation it already holds gets `unchanged`, however often it asks. */
    'project-snapshot'(): SandboxSnapshot {
      const doc = docOf.get(link);
      if (!doc || !docs.has(doc)) throw new Error('Sandbox document is not connected');
      const snapshot = snapshots.read();
      if (doc.delivered === snapshot.generation) return { generation: snapshot.generation, unchanged: true };
      doc.delivered = snapshot.generation;
      return snapshot;
    },
    'extensions-list'() { return host.api.extensions.list().map(({ dir: _dir, ...rest }) => rest); },
    log(level: unknown, message: unknown, data: unknown) { const now = Date.now(); if (now - logWindow >= 1000) { logWindow = now; logCount = 0; } if (++logCount > 50) return; if (level === 'info' || level === 'warn' || level === 'error') host.api.log(level, String(message).slice(0, 4096), ...(Array.isArray(data) ? data : [])); },
    'runtime-error': (error: { message: string }) => deps.reportRuntimeError(record.id, new Error(error?.message)),
    /* A blocked load already did no harm, so live it is logged, never
       counted toward the auto-disable rule. Each open panel replays activate
       in its own document, so one request can surface once per document: each
       distinct key is heard once per extension session, and at most
       MAX_VIOLATION_KEYS are kept. The publish-time sandbox check (observer)
       still fails on any. */
    'csp-violation': (event: { directive?: unknown; blockedURI?: unknown }) => {
      if (violations.size >= MAX_VIOLATION_KEYS) return;
      const directive = String(event?.directive ?? '').slice(0, 64);
      const blockedUri = String(event?.blockedURI ?? '').slice(0, 512);
      const key = `${directive} ${blockedUri}`;
      if (violations.has(key)) return;
      violations.add(key);
      if (observer?.csp) { observer.csp(directive, blockedUri); return; }
      host.api.log('warn', `The sandbox blocked ${blockedUri || 'a request'} (${directive})${violations.size === MAX_VIOLATION_KEYS ? '; further blocked requests are not logged' : ''}`);
    },
    /* Trusted-only reach and sync reads of async results (shim-api.ts). Live,
       they only warn once per member: the throw itself already surfaced. */
    'sandbox-report': (event: { kind?: unknown; member?: unknown }) => {
      const member = typeof event?.member === 'string' ? event.member.slice(0, 120) : '';
      if (!member) return;
      if (event.kind === 'permission') {
        if (observer?.permission) observer.permission(member);
        else if (!warned.has(`p:${member}`)) {
          warned.add(`p:${member}`);
          host.api.log('warn', `api.${member} needs full access and is unavailable in the sandbox`);
        }
      } else if (event.kind === 'async') {
        if (observer?.asyncMisuse) observer.asyncMisuse(member);
        else if (!warned.has(`a:${member}`)) { warned.add(`a:${member}`); host.api.log('warn', `api.${member} returns a Promise in the sandbox; await it (apiVersion 3)`); }
      }
    }
  });
  const warned = new Set<string>();
  let stopForBudget = (): void => {};
  const runtimeLink = { registrations } as { rpc: Rpc; registrations: Map<string, Disposable> };
  const rpc = createRpc(channel.port1, {
    ...shared(runtimeLink, true),
    activated: () => activated(),
    'activation-error': (error: { message: string }) => rejected(new Error(error.message))
  }, 10_000, { budget, unmetered: importedFile, onSustainedLimit: () => stopForBudget(), onRemoteHandleRelease: id => remoteHandles.delete(id) });
  runtimeLink.rpc = rpc;
  /* Data plane (docs/sandbox-data-plane.md §3). Each document holds a small
     SandboxState; state-changing kernel events and subscribed occurrences
     mark a flush, and one flush per microtask sends each document at most
     one `tick(delta, events)`, or nothing when neither changed for it. The
     project is only ever pulled (`project-snapshot`, shared snapshots). */
  const snapshots = projectSnapshots(kernel, deps.project);
  const docs = new Set<PlaneDoc>();
  const docOf = new WeakMap<object, PlaneDoc>();
  /* The selection and its JSON are the kernel's shared copy (ProjectSnapshots.selection),
     so a frame that only moved `time` costs nothing proportional to it. */
  const stateNow = (): { state: SandboxState; selection: string } => {
    const selection = snapshots.selection();
    return { state: { time: host.api.project.time(), playing: host.api.project.playing(), revision: host.api.project.revision(),
      generation: snapshots.generation, selection: selection.value as Selection | null }, selection: selection.json };
  };
  const openDoc = (link: object, docRpc: Rpc): SandboxState => {
    const { state, selection } = stateNow();
    const doc: PlaneDoc = { rpc: docRpc, sent: state, selection, interest: new Map(), pending: [], delivered: null };
    docs.add(doc); docOf.set(link, doc);
    return state;
  };
  let flushQueued = false;
  const schedule = (): void => { if (flushQueued || disposed) return; flushQueued = true; queueMicrotask(flush); };
  function flush(): void {
    flushQueued = false;
    if (disposed || !docs.size) return;
    const { state, selection } = stateNow();
    for (const doc of docs) {
      const delta: Partial<Record<keyof SandboxState, unknown>> = {};
      for (const key of STATE_KEYS) if (doc.sent[key] !== state[key]) delta[key] = state[key];
      if (doc.selection !== selection) delta.selection = state.selection;
      const events = coalesceEvents(doc.pending);
      doc.pending = [];
      if (!events.length && !Object.keys(delta).length) continue;
      doc.sent = state; doc.selection = selection;
      try { doc.rpc.notify('tick', delta, events); sandboxStats().ticks += 1; } catch { /* document closing */ }
    }
  }
  /* The document counts its own listeners; the kernel keeps one subscription
     per document and name, however often the name is registered. */
  const interest = (doc: PlaneDoc, event: string): Disposable => {
    let entry = doc.interest.get(event);
    if (!entry) {
      const source = HOST_EVENTS.has(event) ? event === 'theme' ? 'theme:changed' : event : `ext:${record.id}:${event}`;
      const off = host.api.events.on(source as Parameters<typeof host.api.events.on>[0], (payload: unknown) => {
        if (!docs.has(doc)) return;
        let forwarded: unknown;
        try { forwarded = plain(HOST_EVENTS.has(event) ? parseHostEvent(event, payload) : payload); }
        catch { return; }
        doc.pending.push([event, forwarded]); schedule();
      });
      entry = { count: 0, off };
      doc.interest.set(event, entry);
    }
    entry.count += 1;
    const held = entry;
    let released = false;
    return { dispose() {
      if (released) return; released = true;
      if (--held.count > 0 || doc.interest.get(event) !== held) return;
      doc.interest.delete(event); held.off.dispose();
    } };
  };
  /* What `effects.list()`, `commands.has()` and the like read in the
     sandbox. `sentCatalogs` holds the JSON of the copy each document last got:
     a view mounted since a change already has it, the runtime may not. */
  const sentCatalogs = new WeakMap<object, string>();
  const catalogNow = (): NonNullable<SandboxInit['catalog']> => {
    const catalog = {
      effects: plain(reg.effects.list()) as Array<Record<string, unknown>>,
      transitions: plain(reg.transitions.list()) as Array<Record<string, unknown>>,
      layers: plain(reg.layerTypes.list()) as Array<Record<string, unknown>>,
      theme: plain(reg.themes.list()) as Array<Record<string, unknown>>,
      keybindings: plain(reg.listBindings()) as Array<Record<string, unknown>>,
      commands: reg.commands.list().map(({ run: _run, when: _when, ...entry }) => entry),
      panels: reg.panels.list().map(({ id, title, icon }) => ({ id, title, icon })),
      status: reg.status.list().map(({ id, title, side }) => ({ id, title, side }))
    };
    return catalog;
  };
  const initFor = (state: SandboxState, link: object): SandboxInit => {
    const catalog = catalogNow();
    sentCatalogs.set(link, JSON.stringify(catalog));
    return { id: record.id, apiVersion: manifest.apiVersion ?? 1, manifest, vars,
      theme: themeSnapshot(kernel), activeTheme: kernel.theme.activeId, state,
      bundleUrl: sandboxBundleUrl(base, record.id, record.bundleUrl), catalog };
  };
  const links = new Set<ViewLink>();
  const broadcast = (method: string, value: unknown): void => {
    for (const link of links) { try { link.rpc.notify(method, value); } catch { /* view closing */ } }
  };
  const views: ViewHost = {
    src: panelId => test?.onViewInit ? null : sandboxDocumentUrl(base, record.id, perms, panelId),
    init: link => initFor(openDoc(link, link.rpc), link),
    detach: link => { const doc = docOf.get(link); if (doc) docs.delete(doc); docOf.delete(link); dismissToasts(link); },
    theme: () => viewTheme(kernel),
    keys: () => keyTable(reg, record.id),
    budget,
    focus: (focused, field) => bridge()?.sandboxFocus?.({ focused, field, extensionId: record.id }),
    budgetExceeded: () => stopForBudget(),
    releaseRemoteHandle: id => remoteHandles.delete(id),
    forwardKey: payload => {
      const chord = chordOfEvent(payload);
      if (!chord) return;
      if (chord === 'escape') { (deps.pm as { closeMenus?: () => void }).closeMenus?.(); return; }
      if (chord === 'tab' || chord === 'shift+tab') return;
      for (const binding of reg.bindingsFor(chord)) {
        if (binding.ownerId !== record.id || !ownId(record.id, binding.command) || reg.commands.topEntry(binding.command)?.ownerId !== record.id) continue;
        if (payload.field && !binding.inFields || payload.repeat && !binding.repeat) continue;
        let running: unknown;
        ownRun = true;
        try { running = host.api.commands.run(binding.command, ...(binding.args ?? [])); } finally { ownRun = false; }
        void Promise.resolve(running).catch(error => deps.reportRuntimeError(record.id, error));
        break;
      }
    },
    outsideClick: () => (deps.pm as { closeMenus?: () => void }).closeMenus?.(),
    links,
    connectRuntime: (panelId, token, port) => rpc.notify('mountPanel', panelId, token, port, rpcTransfers(port)),
    disconnectRuntime: token => { try { rpc.notify('unmountPanel', token); } catch { /* runtime already gone */ } },
    handlers: link => shared(link, false),
    report: error => deps.reportRuntimeError(record.id, error),
    ...(observer?.view ? { state: observer.view.bind(observer) } : {}),
    ...(test?.onViewInit ? { post: test.onViewInit } : {})
  };
  /* Keys and theme follow the host while views are open. */
  let keysQueued = false;
  const keysOff = reg.keybindings.onChange(() => {
    if (keysQueued || !links.size) return; keysQueued = true;
    queueMicrotask(() => { keysQueued = false; broadcast('keys', keyTable(reg, record.id)); });
  });
  let themeQueued = false;
  const themeWatch = typeof MutationObserver === 'function' ? new MutationObserver(() => {
    if (themeQueued || !links.size) return; themeQueued = true;
    requestAnimationFrame(() => { themeQueued = false; broadcast('theme', viewTheme(kernel)); });
  }) : null;
  themeWatch?.observe(document.documentElement, { attributes: true, attributeFilter: ['style', 'data-theme'] });
  let disposed = false;
  let watchdog: { dispose(): void } | null = null;
  /* Removing a spinning sandbox's iframe does not stop its process: main
     kills it, while the frames still exist to find it by. */
  const terminate = async (): Promise<void> => { try { await (external ? external.terminate() : bridge()?.sandboxTerminate?.(record.id)); } catch { /* disposing still frees the kernel side */ } };
  const dispose = (): void => {
    if (disposed) return; disposed = true;
    watchdog?.dispose();
    keysOff.dispose(); themeWatch?.disconnect(); docs.clear();
    // Registrations first: panel views tell the runtime to close their ports.
    for (const item of registrations.values()) item.dispose(); registrations.clear();
    dismissToasts(null);
    try { rpc.notify('dispose'); } catch { /* already closed */ }
    rpc.close();
    host.disposeAll(); frame.remove();
    // A contained process does not end with a frame: stop it.
    if (external) void terminate();
  };
  stopForBudget = () => { deps.reportRuntimeError(record.id, new Error('exceeded the sandbox message budget')); dispose(); };
  for (const event of STATE_EVENTS) host.api.events.on(event, schedule);
  host.api.events.on('theme:changed', () => { try { rpc.notify('theme', themeSnapshot(kernel)); } catch { /* disposed */ } });
  /* Other extensions come and go after this one's init: every document gets
     the new catalog, once per burst and only when it changed. It goes out
     before the flush that delivers the same kernel event, so a listener
     reads lists at least as new as that event. */
  let catalogQueued = false;
  const refreshCatalog = (): void => {
    if (catalogQueued || disposed) return; catalogQueued = true;
    queueMicrotask(() => {
      catalogQueued = false;
      if (disposed) return;
      const catalog = catalogNow();
      const json = JSON.stringify(catalog);
      for (const link of [runtimeLink, ...links]) {
        if (sentCatalogs.get(link) === json) continue;
        sentCatalogs.set(link, json);
        try { link.rpc.notify('catalog', catalog); } catch { /* document closing */ }
      }
    });
  };
  for (const event of CATALOG_EVENTS) host.api.events.on(event, refreshCatalog);
  /* The caller hears one outcome within the budget: the probe that tells a
     spinning runtime from a waiting one comes out of it too. */
  const budgetMs = test?.timeoutMs !== undefined && test.timeoutMs > 0 ? test.timeoutMs : 10_000;
  const probeMs = isolated ? Math.min(1_500, budgetMs / 4) : 0;
  let timedOut = false, booted = false;
  try {
    host.setActivating(true);
    const loaded = new Promise<void>((resolve, reject) => {
      if (external) external.loaded.then(resolve, reject);
      else frame.addEventListener('load', () => resolve(), { once: true });
      frame.addEventListener('error', () => reject(new Error('Sandbox document failed to load')), { once: true });
    });
    const timeout = setTimeout(() => { timedOut = true; const error = new Error('Sandbox document or activation timed out'); rejected(error); frame.dispatchEvent(new Event('error')); }, budgetMs - probeMs);
    if (!external) document.body.append(frame);
    try {
      await loaded;
      const init = initFor(openDoc(runtimeLink, rpc), runtimeLink);
      if (external) external.start(channel.port2, init);
      else if (test?.onPostInit) test.onPostInit(channel.port2, init);
      else frame.contentWindow?.postMessage({ t: 'init', ...init }, '*', [channel.port2]);
      /* The shim answers pings before it imports the bundle, so an answer
         proves the runtime booted: only then can a later silence be the
         extension's. A document that never loaded, an error page served in
         its place, or one that never took init answers nothing. */
      void rpc.call('ping').then(() => { booted = true; }, () => {});
      await ready;
    } finally { clearTimeout(timeout); }
    host.setActivating(false);
    // A spinning or crashed runtime stops answering.
    if (isolated) watchdog = watchSandbox({ ping: () => rpc.call('ping'), onUnresponsive: () => void (async () => {
      await terminate();
      dispose();
      deps.reportRuntimeError(record.id, Object.assign(new Error('stopped responding'), { code: 'sandbox_fatal' }));
    })() });
    return { handle: host, dispose };
  } catch (error) {
    host.setActivating(false);
    /* A runtime still answering pings was only waiting (a slow fetch in
       activate); an isolated one that booted and no longer does is spinning
       and would spin again next launch, so it is killed and turned off. The
       caller reports it (sandbox_fatal), once. */
    const spinning = isolated && timedOut && booted && !await Promise.race([rpc.call('ping').then(() => true, () => false), new Promise<boolean>(resolve => setTimeout(() => resolve(false), probeMs))]);
    if (spinning) await terminate();
    dispose();
    throw spinning ? Object.assign(new Error('stopped responding while activating'), { code: 'sandbox_fatal' }) : error;
  }
}
