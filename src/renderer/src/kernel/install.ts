/*
 * Composition root for the kernel inside the running app.
 *
 * `installKernel(PM)` is the ONLY module in kernel/ that knows the legacy PM
 * registry exists: it reads capabilities off PM, packages them as `HostDeps`,
 * and hands the result to host.ts/loader.ts, which stay legacy-free and testable.
 *
 * Every PM lookup is optional-chained on purpose — the kernel must install
 * against a half-built PM (tests, early boot, a legacy engine that failed to
 * install) rather than throwing and taking the app down with it.
 */
import type {
  Disposable,
  EditCommand,
  EditMeta,
  EditResult,
  ExtensionRecord,
  ExtensionsAPI,
  MenuContribution,
  PowermoveAPI,
  Project,
  ProjectAPI,
  Selection,
  StorageAPI,
  UIAPI
} from './api';
import type { ExtensionsBridge } from '../../../shared/extensions';
import { createExtensionAPI, type ExtensionHandle, type HostDeps, type PanelsBackend } from './host';
import { createLoader, type BuiltinFactory, type Loader } from './loader';
import { createKernel, runKernelCommand, type Kernel } from './registries';
import { records as storeRecords } from './extensions.svelte';
import { installRuntimeGlobals } from './runtime-globals';
import { installKernelSignals } from './signals.svelte';
import { DEFAULT_THEME, installThemeApply, type ThemeApplyHandle } from './theme-apply';

type LegacyPM = Record<string, any>;

export interface InstalledKernel extends Kernel {
  /** Set by `bootExtensions`; null until then. */
  loader: Loader | null;
  readonly deps: Omit<HostDeps, 'reportRuntimeError'>;
  readonly bridge: ExtensionsBridge | null;
  /** DOM side of the active theme (tokens, root attributes, injected css). */
  readonly themeApply: ThemeApplyHandle;
  /** Re-activate the persisted theme. Called by `bootExtensions` once built-ins have registered theirs. */
  restoreTheme(): void;
  /** Scoped API for a non-extension owner id (built-in registration from legacy code). */
  api(id: string): PowermoveAPI;
  /** Release the key listener, bus subscriptions, and every scoped API. */
  uninstall(): void;
}

const STORE_PREFIX = 'ext.';

/* ── PM-backed capability adapters ───────────────────────── */

function makeProject(PM: LegacyPM): ProjectAPI {
  const selection = (): Selection => ({
    layers: [...(PM?.sel?.layers ?? [])],
    keys: [...(PM?.sel?.keys ?? [])].filter((key: unknown): key is string => typeof key === 'string'),
    chan: PM?.sel?.chan ?? null
  });
  return {
    get: () => PM?.proj as Project,
    revision: () => Number(PM?.proj?.revision ?? 0),
    apply: (commands: EditCommand | EditCommand[], meta?: EditMeta): EditResult =>
      (PM?.Edit?.apply?.(commands, meta) as EditResult | undefined) ?? { ok: false, message: 'editing engine unavailable' },
    selection,
    select: (layerIds, add) => PM?.selectLayers?.(layerIds, add),
    time: () => Number(PM?.time ?? 0),
    setTime: (t) => PM?.setTime?.(t),
    play: () => PM?.play?.(),
    pause: () => PM?.pause?.(),
    playing: () => !!PM?.playing,
    undo: () => PM?.hist?.undo?.(),
    redo: () => PM?.hist?.redo?.(),
    snapshot: async (t, maxWidth) => (await PM?.Export?.snapshot?.(t ?? PM?.time ?? 0, maxWidth ?? 480)) ?? ''
  };
}

function makeUI(PM: LegacyPM): UIAPI {
  return {
    toast: (text, opts) => PM?.toast?.(text, opts?.sticky ? 8000 : 2200, opts ?? {}),
    confirm: (title, body) =>
      new Promise<boolean>((resolve) => {
        let settled = false;
        const done = (value: boolean): void => {
          if (settled) return;
          settled = true;
          resolve(value);
        };
        const handle = PM?.modal?.({
          title,
          body: body ?? '',
          actions: [
            { label: 'Cancel', run: () => done(false) },
            { label: 'OK', pri: true, run: () => done(true) }
          ],
          onClose: () => done(false)
        });
        if (!handle) done(false);
      }),
    menu: (anchor, items: MenuContribution[]) => {
      if (anchor && typeof (anchor as HTMLElement).getBoundingClientRect === 'function') PM?.menu?.(anchor, items);
      else {
        const point = anchor as { x: number; y: number };
        PM?.menu?.(document.body, items, { x: point?.x ?? 0, y: point?.y ?? 0 });
      }
    },
    modal: (opts) => PM?.modal?.(opts) ?? { close: () => {}, body: document.createElement('div') },
    icon: (name) => {
      const icon = PM?.icon?.(name);
      return typeof icon === 'string' ? icon : String(icon?.outerHTML ?? icon ?? '');
    }
  };
}

/** One JSON object per extension under `ext.<id>` — read-modify-write per call. */
function makeStorage(PM: LegacyPM): (id: string) => StorageAPI {
  const read = (id: string): Record<string, unknown> => {
    const value = PM?.store?.get?.(STORE_PREFIX + id, {});
    return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
  };
  const write = (id: string, next: Record<string, unknown>): void => void PM?.store?.set?.(STORE_PREFIX + id, next);
  return (id: string): StorageAPI => ({
    get: <T = unknown,>(key: string): T | undefined => read(id)[key] as T | undefined,
    set: (key, value) => write(id, { ...read(id), [key]: value }),
    delete: (key) => {
      const next = read(id);
      delete next[key];
      write(id, next);
    }
  });
}

/**
 * Panel visibility, expressed the way the legacy shell expresses it: the
 * workspace owns which panels are mounted, so open/close go through
 * `PM.WS.mutate` and the layout re-renders from the new workspace. `close`
 * HIDES rather than removes — the spec (size, dock, order) survives so
 * reopening restores the panel where the user left it. Mirrors
 * panels/register-shader.ts's `PM.openShaderEditor`.
 */
function makePanelsBackend(PM: LegacyPM, kernel: Kernel): PanelsBackend {
  const current = (): unknown => PM?.WS?.current;
  return {
    open: (id, dock) => {
      if (!PM?.Layout?.hasPanel?.(current(), id)) {
        PM?.WS?.mutate?.((workspace: any) => {
          /* A hidden panel remembers its dock, index and size — restore beats
             a fresh add, which would drop all three. */
          if (PM?.Layout?.restorePanel?.(workspace, id)) return;
          PM?.Layout?.addPanel?.(workspace, id, dock ?? 'center');
        });
      }
      PM?.Layout?.refresh?.(id);
    },
    close: (id) => {
      const hide = PM?.Layout?.hidePanel ?? PM?.Layout?.removePanel;
      if (!hide) return;
      PM?.WS?.mutate?.((workspace: unknown) => hide(workspace, id));
    },
    isOpen: (id) => !!PM?.Layout?.hasPanel?.(current(), id),
    refresh: (id) => PM?.Layout?.refresh?.(id),
    list: () => [...new Set([...kernel.panels.ids(), ...Object.keys(PM?.PANELS ?? {})])]
  };
}

function makeExtensionsAPI(PM: LegacyPM, bridge: ExtensionsBridge | null, getLoader: () => Loader | null): ExtensionsAPI {
  return {
    list: (): ExtensionRecord[] => storeRecords(),
    setEnabled: async (id, enabled) => void (await bridge?.setEnabled({ id, enabled })),
    remove: async (id) => void (await bridge?.remove({ id })),
    reload: async (id) => {
      if (bridge) await bridge.reload({ id });
      else await getLoader()?.reload(id);
    },
    reveal: async (id) => void (await bridge?.reveal({ id })),
    requestFix: (id) => {
      if (typeof PM?.requestExtensionFix === 'function') PM.requestExtensionFix(id);
      else PM?.cmd?.('agent');
    }
  };
}

function resolveBridge(PM: LegacyPM): ExtensionsBridge | null {
  const candidate = (globalThis as Record<string, any>)?.powermove?.extensions ?? PM?.extensionsBridge ?? null;
  if (candidate && typeof candidate.list === 'function' && typeof candidate.onChanged === 'function') return candidate as ExtensionsBridge;
  return null;
}

/* ── install ─────────────────────────────────────────────── */

export function installKernel(PM: LegacyPM): InstalledKernel {
  const kernel = createKernel();
  installRuntimeGlobals();

  const bridge = resolveBridge(PM);
  /* Boxed so `deps` and `api()` see the loader that `bootExtensions` installs
     later without recapturing a stale binding. */
  const box: { loader: Loader | null } = { loader: null };

  const deps: Omit<HostDeps, 'reportRuntimeError'> = {
    pm: PM,
    ui: makeUI(PM),
    project: makeProject(PM),
    storage: makeStorage(PM),
    extensions: makeExtensionsAPI(PM, bridge, () => box.loader),
    panelsBackend: makePanelsBackend(PM, kernel),
    paletteOpen: (query?: string) => {
      const palette = PM?.palette;
      if (typeof palette?.open === 'function') palette.open(query);
      else if (typeof palette === 'function') palette(query);
      else PM?.cmd?.('palette');
    }
  };

  const handles = new Map<string, ExtensionHandle>();
  const subscriptions: Array<() => void> = [];

  /* Route through PM.cmd when present: it is the app's public command seam
     (native menu IPC and tests interpose on it). Falls back to the kernel. */
  const keyListener = kernel.installKeyListener((command, args) =>
    typeof PM?.cmd === 'function' ? PM.cmd(command, ...(args ?? [])) : runKernelCommand(kernel, command, args)
  );
  subscriptions.push(() => keyListener.dispose());

  const signals = installKernelSignals(kernel);
  subscriptions.push(() => signals.dispose());

  /* Themes: the kernel owns which theme is active and paints it; legacy
     `PM.theme` (app.ts) still owns the light/dark preference and its
     persistence, so the two are bridged rather than duplicated. The default
     definition arrives from the theme-default built-in during extension boot. */
  const storedScheme = PM?.store?.get?.('themeMode', 'system');
  if (storedScheme === 'light' || storedScheme === 'dark' || storedScheme === 'system') kernel.theme.scheme = storedScheme;
  const themeApply = installThemeApply(kernel);
  subscriptions.push(() => themeApply.dispose());

  const activateThemeBase = kernel.activateTheme.bind(kernel);
  const setSchemeBase = kernel.setScheme.bind(kernel);

  /* Legacy bus → kernel events. One subscription per legacy topic; the kernel
     event names are the stable ones extensions program against. */
  const bus = PM?.bus;
  const on = (event: string, fn: (...args: any[]) => void): void => {
    const off = bus?.on?.(event, fn);
    if (typeof off === 'function') subscriptions.push(off);
  };
  const projectKinds: Record<string, 'values' | 'structure' | 'project' | 'assets' | 'library' | 'history'> = {
    layers: 'structure',
    project: 'project',
    assets: 'assets',
    library: 'library',
    history: 'history'
  };
  for (const [event, kind] of Object.entries(projectKinds)) on(event, () => kernel.events.emit('project:changed', { kind }));
  on('sel', () => kernel.events.emit('selection', deps.project.selection()));
  on('time', (t: number) => kernel.events.emit('time', Number(t ?? PM?.time ?? 0)));
  on('transport', () => kernel.events.emit('transport', { playing: !!PM?.playing }));
  on('layout', () => kernel.events.emit('layout', undefined));

  const syntheticRecord = (id: string): ExtensionRecord => ({
    id,
    scope: 'builtin',
    manifest: { id, name: id, version: '1.0.0', apiVersion: 1 },
    dir: `builtin:${id}`,
    enabled: true,
    bundleUrl: null,
    bundleHash: null,
    health: { state: 'ok' },
    updatedAt: 0
  });

  const installed = kernel as InstalledKernel;
  Object.defineProperty(installed, 'loader', {
    enumerable: true,
    get: () => box.loader,
    set: (next: Loader | null) => void (box.loader = next)
  });
  Object.assign(installed, {
    deps,
    bridge,
    themeApply,
    activateTheme(id: string): void {
      activateThemeBase(id);
      PM?.store?.set?.('activeTheme', id);
    },
    setScheme(mode: 'light' | 'dark' | 'system'): void {
      /* Legacy owns persistence and the <html data-theme> flag. `PM.theme.apply`
         writes back into `kernel.theme.scheme` and emits `theme:changed`
         itself, so calling both would repaint twice. */
      if (typeof PM?.theme?.apply === 'function') PM.theme.apply(mode);
      else setSchemeBase(mode);
    },
    restoreTheme(): void {
      const id = PM?.store?.get?.('activeTheme', DEFAULT_THEME.id);
      installed.activateTheme(typeof id === 'string' && kernel.themes.has(id) ? id : DEFAULT_THEME.id);
    },
    api(id: string): PowermoveAPI {
      let handle = handles.get(id);
      if (!handle) {
        handle = createExtensionAPI(kernel, syntheticRecord(id), {
          ...deps,
          reportRuntimeError: (ownerId, error) => box.loader?.reportRuntimeError(ownerId, error)
        });
        handles.set(id, handle);
      }
      return handle.api;
    },
    uninstall(): void {
      for (const off of subscriptions.splice(0)) {
        try {
          off();
        } catch (error) {
          console.error('[kernel] bus unsubscribe failed', error);
        }
      }
      for (const handle of handles.values()) handle.disposeAll();
      handles.clear();
      kernel.dispose();
    }
  });

  PM.Kernel = installed;
  return installed;
}

/**
 * Create the loader for `installKernel`'s deps and boot it. Kept separate so the
 * kernel can be installed (and used by legacy registration) before any
 * extension code runs.
 */
export async function bootExtensions(installed: InstalledKernel, builtins: Record<string, BuiltinFactory> = {}): Promise<Loader> {
  const loader = createLoader({
    kernel: installed,
    bridge: installed.bridge,
    builtins,
    deps: installed.deps
  });
  installed.loader = loader;
  const booting = loader.boot();
  await loader.builtinsReady;
  /* Built-ins have now registered their themes, so the persisted id resolves. */
  installed.restoreTheme();
  /* ToolbarMount retries its lazy panel lookup on layout. It mounted before
     extensions booted, so retry as soon as the bundled toolbar is available. */
  (installed.deps.pm as Record<string, any>)?.bus?.emit?.('layout');
  await booting;
  /* Extensions may add or replace layout contributions during full boot. */
  (installed.deps.pm as Record<string, any>)?.bus?.emit?.('layout');
  return loader;
}

export type { Disposable };
