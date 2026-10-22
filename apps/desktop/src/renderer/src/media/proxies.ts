/*
 * The renderer side of proxies (plan section 14, appendix D, Phase 4a).
 *
 * `PM.EngineProxies` is main's proxy service for this window (the engine
 * connection installs it from `host.nativeEngine.proxies` with "Use New
 * Engine" on). This controller is the one place the UI talks to it:
 *
 * - the project's intent (`project.media.proxies`, each asset's
 *   `alwaysPlayOriginal`) is edited here as undoable writes, and sent to
 *   main with what it needs to key each asset's proxy, whenever the
 *   project, its assets, or the intent change (undo and redo included);
 * - the asset menu's Create, Rebuild and Delete Proxy go through it;
 * - the generated media location (a setting of this Mac) and the preview
 *   memory limit (the engine's cached preview) pass through;
 * - main's snapshots become each asset's state and one status notice,
 *   "Creating proxies · 3 of 12" or "Proxies paused · Low disk space",
 *   left as it is during playback.
 *
 * With the service absent (the current engine) `PM.Proxies` is null and
 * no proxy control shows.
 */
import {
  ENGINE_PROXIES_CHANGED,
  proxyEligible,
  proxyIntentOf,
  storedProxyIntent,
  type AssetProxyState,
  type EngineProxiesBridge,
  type GeneratedMediaLocation,
  type ProjectProxyIntent,
  type ProxyAssetDescriptor,
  type ProxySnapshot
} from './proxies-types';

export type ProjectIntentField = 'create' | 'size' | 'format';

export interface ProxyController {
  readonly bridge: EngineProxiesBridge;
  /** The open project's intent, as stored (absent fields are defaults). */
  intent(): ProjectProxyIntent;
  /** One undo step per change. */
  setIntent(patch: Partial<Pick<ProjectProxyIntent, ProjectIntentField>>): void;
  alwaysPlayOriginal(assetId: string): boolean;
  setAlwaysPlayOriginal(assetId: string, on: boolean): void;
  /** What main last said about an asset's proxy. */
  assetState(assetId: string): AssetProxyState;
  /** Whether the asset can have a proxy at all (video, image sequences). */
  eligible(assetId: string): boolean;
  create(assetId: string): Promise<void>;
  rebuild(assetId: string): Promise<void>;
  remove(assetId: string): Promise<void>;
  location(): Promise<GeneratedMediaLocation>;
  setLocation(location: GeneratedMediaLocation): Promise<GeneratedMediaLocation>;
  /** Shows the folder panel; the location in force afterwards (unchanged when cancelled). */
  chooseFolder(): Promise<GeneratedMediaLocation | null>;
  setPreviewMemory(bytes: number): void;
  /** Called after each snapshot and intent change. */
  onChange(listener: () => void): () => void;
  /** Sends the intent and asset descriptors now (tests, project switches). */
  flush(): Promise<void>;
  dispose(): void;
}

type Registry = Record<string, any>;

const STATUS_KEY = 'media-proxies';
const SYNC_DELAY_MS = 250;

/** The content hash the media store and pm-media agree on (BLAKE3, hex). */
const HASH = /^[0-9a-f]{64}$/;

/** What main needs to find and key one asset's proxy. */
export function assetDescriptor(meta: any, recordHash?: (key: string) => string | null | undefined): ProxyAssetDescriptor {
  const out: ProxyAssetDescriptor = {};
  const hash = [meta?.contentHash, meta?.link?.hash, typeof meta?.storageKey === 'string' ? recordHash?.(meta.storageKey) : null]
    .find((value) => typeof value === 'string' && HASH.test(value));
  if (hash) out.contentHash = hash;
  if (typeof meta?.kind === 'string') out.kind = meta.kind;
  if (typeof meta?.name === 'string') out.name = meta.name;
  const source = typeof meta?.link?.path === 'string' ? meta.link.path : typeof meta?.sourcePath === 'string' ? meta.sourcePath : null;
  if (source) out.sourcePath = source;
  if (meta?.imageSequence) out.imageSequence = true;
  if (meta?.alwaysPlayOriginal === true) out.alwaysPlayOriginal = true;
  return out;
}

export function assetDescriptors(project: any, recordHash?: (key: string) => string | null | undefined): Record<string, ProxyAssetDescriptor> {
  const out: Record<string, ProxyAssetDescriptor> = {};
  for (const [id, meta] of Object.entries<any>(project?.assets ?? {})) {
    if (meta && typeof meta === 'object') out[id] = assetDescriptor(meta, recordHash);
  }
  return out;
}

/** The status notice for a snapshot's activity, or null for none. */
export function proxyStatus(activity: ProxySnapshot['activity'] | null | undefined): { text: string; progress: number | null } | null {
  if (!activity) return null;
  if (activity.paused === 'lowDisk') return { text: 'Proxies paused · Low disk space', progress: null };
  const creating = activity.creating;
  if (!creating || creating.total <= 0) return null;
  const done = Math.min(creating.done, creating.total);
  return { text: `Creating proxies · ${done + (done < creating.total ? 1 : 0)} of ${creating.total}`, progress: done / creating.total };
}

/** A rejected call's message, for "Couldn't … · <message>". */
function reason(error: unknown): string {
  const message = error instanceof Error ? error.message : typeof error === 'string' ? error : '';
  return message.replace(/^Error invoking remote method '[^']+': (?:Error: )?/, '').trim() || 'Try again';
}

export function createProxyController(PM: Registry, bridge: EngineProxiesBridge): ProxyController {
  let snapshot: ProxySnapshot | null = null;
  let disposed = false;
  let sent: { project: string; signature: string } | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let pendingStatus = false;
  let shownStatus: string | null = null;
  const listeners = new Set<() => void>();
  const offs: Array<() => void> = [];

  const project = (): any => (PM.proj && typeof PM.proj.id === 'string' ? PM.proj : null);
  const notify = (): void => { for (const listener of [...listeners]) listener(); };
  const recordHash = (key: string): string | null | undefined => PM.HostMedia?.store?.recordFor?.(key)?.hash;
  const projectPath = (id: string): string | undefined => {
    try {
      const path = PM.projectFileState?.(id, { initialize: false })?.path;
      return typeof path === 'string' && path ? path : undefined;
    } catch {
      return undefined;
    }
  };

  const toastError = (verb: string, assetId: string, error: unknown): void => {
    const name = project()?.assets?.[assetId]?.name || 'this clip';
    PM.toast?.(`Couldn't ${verb} for ${name} · ${reason(error)}`, 6000, { error: true, kind: 'error' });
  };

  /* Status: one keyed notice, replaced in place, left alone while playing. */
  const showStatus = (): void => {
    if (PM.playing) { pendingStatus = true; return; }
    pendingStatus = false;
    const status = snapshot && snapshot.project === project()?.id ? proxyStatus(snapshot.activity) : null;
    if (!status) {
      if (shownStatus) PM.dismissToast?.(STATUS_KEY);
      shownStatus = null;
      return;
    }
    if (shownStatus === status.text && status.progress === null) return;
    shownStatus = status.text;
    PM.toast?.(status.text, 0, {
      key: STATUS_KEY, sticky: true, kind: 'status', dismissible: true,
      ...(status.progress === null ? {} : { progress: status.progress })
    });
  };

  const send = async (force = false): Promise<void> => {
    if (timer) { clearTimeout(timer); timer = null; }
    const current = project();
    if (disposed || !current) return;
    const intent = proxyIntentOf(current);
    const assets = assetDescriptors(current, recordHash);
    const path = projectPath(current.id);
    const signature = JSON.stringify([intent, assets, path ?? null]);
    if (!force && sent && sent.project === current.id && sent.signature === signature) return;
    sent = { project: current.id, signature };
    try {
      await bridge.setIntent(current.id, intent, assets, path);
    } catch (error) {
      // The next change retries.
      if (sent?.signature === signature) sent = null;
      PM.log?.('warn', `[proxies] setIntent failed: ${reason(error)}`);
    }
  };
  const schedule = (): void => {
    if (disposed || timer) return;
    timer = setTimeout(() => void send(), SYNC_DELAY_MS);
  };

  const refresh = async (): Promise<void> => {
    const current = project();
    if (!current) return;
    try {
      const next = await bridge.snapshot(current.id);
      if (!disposed && next?.project === project()?.id) { snapshot = next; showStatus(); notify(); }
    } catch (error) {
      PM.log?.('warn', `[proxies] snapshot failed: ${reason(error)}`);
    }
  };

  offs.push(bridge.subscribe((next) => {
    if (disposed || !next || next.project !== project()?.id) return;
    snapshot = next;
    showStatus();
    notify();
  }));
  const on = (event: string, fn: () => void): void => {
    const off = PM.bus?.on?.(event, fn);
    if (typeof off === 'function') offs.push(off);
  };
  let openId: string | null = project()?.id ?? null;
  const projectChanged = (): void => {
    const id = project()?.id ?? null;
    if (id !== openId) {
      openId = id;
      snapshot = null;
      notify();
      void send(true);
      void refresh();
      showStatus();
      return;
    }
    schedule();
  };
  on('project', projectChanged);
  on('projects:open', projectChanged);
  on('assets', schedule);
  on('transport', () => { if (!PM.playing && pendingStatus) showStatus(); });

  const writeProject = (label: string, fn: () => void): void => {
    PM.hist?.do ? PM.hist.do(label, fn) : fn();
    PM.touch?.();
    PM.bus?.emit?.('project');
    PM.autosave?.();
    void send();
    notify();
  };

  const run = async (verb: string, assetId: string, call: () => Promise<void>): Promise<void> => {
    const current = project();
    if (!current) return;
    // Main keys the proxy by the descriptor it was last sent.
    await send();
    try { await call(); } catch (error) { toastError(verb, assetId, error); }
    void refresh();
  };

  const controller: ProxyController = {
    bridge,
    intent: () => proxyIntentOf(project()),
    setIntent(patch) {
      const current = project();
      if (!current) return;
      const before = proxyIntentOf(current);
      const next = { ...before, ...patch };
      const changed = (['create', 'size', 'format'] as const).find((field) => next[field] !== before[field]);
      if (!changed) return;
      const label = changed === 'create' ? 'Create proxies' : changed === 'size' ? 'Proxy size' : 'Proxy format';
      writeProject(label, () => {
        const media = { ...(current.media || {}) };
        const stored = storedProxyIntent(next);
        if (stored) media.proxies = stored; else delete media.proxies;
        if (Object.keys(media).length) current.media = media; else delete current.media;
      });
    },
    alwaysPlayOriginal: (assetId) => project()?.assets?.[assetId]?.alwaysPlayOriginal === true,
    setAlwaysPlayOriginal(assetId, on) {
      const meta = project()?.assets?.[assetId];
      if (!meta || (meta.alwaysPlayOriginal === true) === on) return;
      writeProject('Always Play Original', () => {
        if (on) meta.alwaysPlayOriginal = true; else delete meta.alwaysPlayOriginal;
      });
      PM.bus?.emit?.('assets');
    },
    assetState(assetId) {
      if (!controller.eligible(assetId)) return { state: 'unsupported' };
      return (snapshot && snapshot.project === project()?.id ? snapshot.assets[assetId] : null) ?? { state: 'none' };
    },
    eligible: (assetId) => proxyEligible(project()?.assets?.[assetId]),
    create: (assetId) => run('create a proxy', assetId, () => bridge.create(project().id, [assetId])),
    rebuild: (assetId) => run('rebuild the proxy', assetId, () => bridge.rebuild(project().id, [assetId])),
    remove: (assetId) => run('delete the proxy', assetId, () => bridge.remove(project().id, [assetId])),
    location: () => bridge.location(),
    setLocation: (location) => bridge.setLocation(location),
    async chooseFolder() {
      const path = await bridge.chooseFolder();
      if (!path) return null;
      return bridge.setLocation({ kind: 'folder', path });
    },
    setPreviewMemory(bytes) {
      const set = bridge.setPreviewMemory;
      if (typeof set !== 'function') return;
      void set.call(bridge, Math.max(0, Math.round(bytes))).catch((error: unknown) => PM.log?.('warn', `[proxies] preview memory: ${reason(error)}`));
    },
    onChange(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    flush: () => send(),
    dispose() {
      if (disposed) return;
      disposed = true;
      if (timer) clearTimeout(timer);
      timer = null;
      for (const off of offs.splice(0)) { try { off(); } catch { /* already gone */ } }
      listeners.clear();
      if (shownStatus) PM.dismissToast?.(STATUS_KEY);
    }
  };

  // The project open now, and the cached preview's budget as stored.
  void send(true);
  void refresh();
  const budget = Number(PM.Memory?.budget?.('preview'));
  if (Number.isFinite(budget)) controller.setPreviewMemory(budget);
  return controller;
}

/** Installs `PM.Proxies` for the window's `PM.EngineProxies`, following it
 *  as the engine connection sets or clears it. */
export function installMediaProxies(PM: Registry, target: EventTarget = window): () => void {
  let current: ProxyController | null = null;
  const sync = (): void => {
    const bridge = PM.EngineProxies as EngineProxiesBridge | undefined;
    if (current && current.bridge === bridge) return;
    current?.dispose();
    current = bridge ? createProxyController(PM, bridge) : null;
    PM.Proxies = current;
    PM.bus?.emit?.('proxies');
  };
  sync();
  target.addEventListener(ENGINE_PROXIES_CHANGED, sync);
  return () => {
    target.removeEventListener(ENGINE_PROXIES_CHANGED, sync);
    current?.dispose();
    current = null;
    PM.Proxies = null;
  };
}
