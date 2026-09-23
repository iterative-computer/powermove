/*
 * The per-extension `PowermoveAPI` factory.
 *
 * Two invariants make the loader's recovery story work:
 *
 *  1. EVERY registration is owned. Each call returns a Disposable that is also
 *     collected here, so `handle.dispose()` releases the extension's whole
 *     footprint whether or not it kept the Disposables itself.
 *  2. EVERY extension-provided callback is wrapped by `guard`. A throw becomes a
 *     logged, attributed runtime error instead of a broken host callsite — this
 *     is what feeds the loader's 2-failures-in-10s auto-disable rule.
 *
 * Host capabilities arrive through `HostDeps` rather than imports: this module
 * has zero legacy dependencies, so it can be unit-tested with a fake PM.
 */
import type {
  AssetsAPI,
  AnimAPI,
  CommandDefinition,
  CommandsAPI,
  Disposable,
  DndAPI,
  EaseAPI,
  EditAPI,
  EffectsAPI,
  ExtensionLayersAPI,
  EventsAPI,
  ExtensionManifest,
  ExtensionRecord,
  ExtensionsAPI,
  GroupsAPI,
  HistoryAPI,
  KeybindingDefinition,
  KeybindingsAPI,
  MenuContribution,
  MenuLocation,
  MenusAPI,
  MediaAPI,
  ModelAPI,
  PaletteAPI,
  PaletteEntry,
  PaletteProvider,
  PanelDefinition,
  PanelsAPI,
  PowermoveAPI,
  ProjectAPI,
  RenderAPI,
  SelectionAPI,
  ServicesAPI,
  Space3DAPI,
  StatusAPI,
  StatusItem,
  StorageAPI,
  ThemeAPI,
  ThemeDefinition,
  TransportAPI,
  TransitionsAPI,
  UIAPI,
  UIStateAPI,
  UtilAPI,
  WorkspaceAPI,
  PanelDock,
  PanelOpenOptions
} from './api';
import type { EffectDefinition, EditCommand, EditMeta, EditResult, TransitionDefinition } from './api';
import type { ExtensionLayerDefinition } from './api';
import type { Component } from 'svelte';
import { chordOfEvent } from './keychord';
import { runKernelCommand, type Kernel } from './registries';
import { mountComponent } from './runtime-globals';
import { performanceMonitor } from '../runtime/performance-monitor';
import { IMPORT_DEFAULTS_SERVICE, validatedImportDefaults } from './import-defaults';
import { bridge as hostBridge } from './bridge';
import { PANEL_FRAME, type PanelFrame } from './panel-frame';

/* Extensions never see the raw bridge (bridge.ts); haptics is the one bit a
   built-in (the viewer's snapping) needs, so it goes through the API. */
const HOST_HAPTIC = Object.freeze({
  alignment(): void {
    try {
      hostBridge()?.haptic?.alignment();
    } catch {
      // Haptics are a nicety; a missing or failing host changes nothing.
    }
  }
});

export interface PanelsBackend {
  open(id: string, dock?: PanelDock | PanelOpenOptions): void;
  /** True when the user closed this panel and the workspace remembers it hidden. */
  isHidden?(id: string): boolean;
  close(id: string): void;
  isOpen(id: string): boolean;
  refresh(id: string): void;
  list(): string[];
}

export interface HostDeps {
  /** The legacy PM registry, surfaced as `api.host.pm` (documented unstable). */
  pm: unknown;
  state: {
    doc: unknown;
    sel: unknown;
    transport: unknown;
    perf: unknown;
  };
  ui: {
    controls: UIAPI['controls'];
    toast: UIAPI['toast'];
    confirm: UIAPI['confirm'];
    menu: UIAPI['menu'];
    modal: UIAPI['modal'];
    icon: UIAPI['icon'];
  };
  project: ProjectAPI;
  anim?: AnimAPI;
  model?: ModelAPI;
  selection?: SelectionAPI;
  groups?: GroupsAPI;
  transport?: TransportAPI;
  history?: HistoryAPI;
  edit?: EditAPI;
  media?: MediaAPI;
  render?: RenderAPI;
  uiState?: UIStateAPI;
  dnd?: DndAPI;
  workspace?: WorkspaceAPI;
  util?: UtilAPI;
  ease?: EaseAPI;
  space3d?: Space3DAPI;
  assets: AssetsAPI;
  storage(id: string): StorageAPI;
  extensions: Omit<ExtensionsAPI, 'fork' | 'setUp'> & Partial<Pick<ExtensionsAPI, 'fork' | 'setUp'>>;
  panelsBackend: PanelsBackend;
  paletteOpen(query?: string): void;
  /** Called for every guarded callback that throws — the loader's error policy hook. */
  reportRuntimeError(id: string, error: unknown): void;
}

export interface ExtensionHandle {
  readonly id: string;
  readonly api: PowermoveAPI;
  /** Release every registration, event handler, and user disposer. */
  disposeAll(): void;
  /** Marks the window during which activate(api) runs. */
  setActivating(on: boolean): void;
  /**
   * Kernel-only (never on the extension API): register a panel whose body is
   * docked by `frame` — the sandboxed extension's view iframe. Host chrome
   * comes from `title`; `header`, `moveSlot`, `headless` and `library.render`
   * do not exist for this variant.
   */
  registerFramePanel(def: FramePanelDefinition, frame: PanelFrame): Disposable;
}

export type FramePanelDefinition = Pick<PanelDefinition, 'id' | 'title' | 'icon' | 'size' | 'min' | 'flush' | 'noscroll'>;

const FALLBACK_MANIFEST = (id: string): ExtensionManifest => ({ id, name: id, version: '0.0.0', apiVersion: 1 });

/** `api.vars` over a frozen copy of the values delivered for one activation. */
export function createVarsAPI(values: Readonly<Record<string, string>> = {}): import('./api').VarsAPI {
  const own = new Map(Object.entries(values).filter(([, value]) => typeof value === 'string'));
  return Object.freeze({
    get: (key: string) => own.get(key),
    has: (key: string) => own.has(key),
    keys: () => [...own.keys()]
  });
}

export function createExtensionAPI(
  kernel: Kernel,
  record: ExtensionRecord,
  deps: HostDeps,
  vars: Readonly<Record<string, string>> = {}
): ExtensionHandle {
  const id = record.id;
  const manifest = record.manifest ?? FALLBACK_MANIFEST(id);
  const disposers: Array<() => void> = [];
  let disposed = false;
  let activating = false;

  const log = (level: 'info' | 'warn' | 'error', message: string, ...data: unknown[]): void => {
    const line = `[ext:${id}] ${message}`;
    if (level === 'error') console.error(line, ...data);
    else if (level === 'warn') console.warn(line, ...data);
    else console.info(line, ...data);
  };

  /** Wrap an extension callback so a throw is attributed, logged, and reported. */
  function guard<A extends unknown[], R>(fn: (...args: A) => R, label: string, fallback: R): (...args: A) => R {
    return (...args: A): R => {
      if (disposed && label !== 'onDispose handler') return fallback;
      const start = performance.now();
      const fail = (error: unknown): R => {
        log('error', `${label} threw`, error);
        if (!disposed) deps.reportRuntimeError(id, error);
        return fallback;
      };
      try {
        const result = fn(...args);
        if (result && typeof (result as any).then === 'function') {
          return Promise.resolve(result).catch(fail) as R;
        }
        return result;
      } catch (error) {
        return fail(error);
      } finally {
        if (record.scope !== 'builtin') performanceMonitor.record({ id: `extension:${id}`, name: `${manifest.name} · ${label}`, kind: 'extension' }, performance.now() - start);
      }
    };
  }

  /* What an extension says is that extension's notice, not the editor's. Stamp
     its identity last so the toast can name the speaker and keep the message
     out of the editor's error family, whatever the extension passed. */
  const extensionUI = {
    ...(deps.ui as UIAPI),
    toast: (text: string, opts?: Parameters<UIAPI['toast']>[1]): void =>
      deps.ui.toast(text, { ...opts, source: { id, name: manifest.name } } as Parameters<UIAPI['toast']>[1])
  } as UIAPI;

  const collect = (disposable: Disposable): Disposable => {
    if (disposed) {
      disposable.dispose();
      return { dispose: () => {} };
    }
    let released = false;
    const release = (): void => {
      if (released) return;
      released = true;
      disposable.dispose();
    };
    disposers.push(release);
    return { dispose: release };
  };

  const extensions: ExtensionsAPI = {
    ...deps.extensions,
    fork: deps.extensions.fork ?? (async () => {
      throw new Error('Forking built-in extensions is unavailable in this host.');
    }),
    setUp: deps.extensions.setUp ?? (() => undefined)
  };

  /* ── panels ────────────────────────────────────────────── */

  const panels: PanelsAPI = {
    register(def: PanelDefinition) {
      if (!def || typeof def.id !== 'string' || !def.id) throw new Error(`[ext:${id}] panels.register requires an id`);
      if (!def.component && typeof def.build !== 'function') throw new Error(`[ext:${id}] panel "${def.id}" needs component or build`);
      const guarded: PanelDefinition = { ...def };
      if (typeof def.build === 'function') {
        const build = def.build;
        guarded.build = guard((body: HTMLElement, inst: { spec: Record<string, unknown> }) => build(body, inst), `panel ${def.id} build`, undefined);
      }
      if (typeof def.header === 'function') {
        const header = def.header;
        guarded.header = guard((el: HTMLElement, inst: Record<string, unknown>) => header(el, inst), `panel ${def.id} header`, undefined);
      }
      if (def.library && typeof def.library.render === 'function') {
        const render = def.library.render;
        guarded.library = {
          ...def.library,
          render: guard((context) => render(context), `panel ${def.id} library preview`, undefined)
        };
      }
      /* A component panel still has to satisfy the host's imperative `build`
         contract (legacy layout calls `def.build(body, inst)`), so mount it
         through the shared Svelte runtime here. One live instance per panel id:
         a rebuild (refresh, override) unmounts the previous one first, and
         disposing the registration unmounts the last one. */
      if (!guarded.build && def.component) {
        const component = def.component;
        let unmountPrevious: (() => void) | null = null;
        const release = (): void => {
          const fn = unmountPrevious;
          unmountPrevious = null;
          fn?.();
        };
        guarded.build = guard(
          (body: HTMLElement, inst: { spec: Record<string, unknown> }) => {
            release();
            unmountPrevious = mountComponent(component as unknown as Component<Record<string, unknown>>, body, {
              panelId: def.id,
              spec: inst?.spec ?? {},
              api
            });
          },
          `panel ${def.id} mount`,
          undefined
        );
        collect({ dispose: release });
      }
      return collect(kernel.panels.register(id, guarded));
    },
    /* The backend merges kernel ids with panels the legacy PM still owns. */
    list: () => deps.panelsBackend.list(),
    open: (panelId, dock) => {
      /* Extensions habitually open their panel from activate(), which runs on
         every launch. If the user closed that panel, their choice wins over the
         extension's boot-time reveal; explicit opens later (commands, menus)
         still bring it back. */
      if (activating && deps.panelsBackend.isHidden?.(panelId)) return;
      deps.panelsBackend.open(panelId, dock);
    },
    close: (panelId) => deps.panelsBackend.close(panelId),
    isOpen: (panelId) => deps.panelsBackend.isOpen(panelId),
    refresh: (panelId) => deps.panelsBackend.refresh(panelId)
  };

  function registerFramePanel(def: FramePanelDefinition, frame: PanelFrame): Disposable {
    if (!def || typeof def.id !== 'string' || !def.id) throw new Error(`[ext:${id}] panels.register requires an id`);
    /* One live view per panel id, like the component variant: a rebuild
       (layout refresh, override swap) disposes the previous view first. */
    let current: { dispose(): void } | null = null;
    const release = (): void => {
      const view = current;
      current = null;
      view?.dispose();
    };
    const panel: PanelDefinition = {
      id: def.id,
      title: typeof def.title === 'string' && def.title ? def.title : def.id,
      ...(typeof def.icon === 'string' ? { icon: def.icon } : {}),
      ...(typeof def.size === 'number' ? { size: def.size } : {}),
      ...(typeof def.min === 'number' ? { min: def.min } : {}),
      ...(def.flush ? { flush: true } : {}),
      ...(def.noscroll ? { noscroll: true } : {}),
      build: guard((body: HTMLElement, inst: { spec: Record<string, unknown> }) => {
        release();
        current = frame.mount(body, { spec: inst?.spec ?? {} });
      }, `panel ${def.id} view`, undefined)
    };
    Object.defineProperty(panel, PANEL_FRAME, { value: { extensionId: id }, enumerable: true });
    const registration = kernel.panels.register(id, panel);
    return collect({ dispose() { registration.dispose(); release(); } });
  }

  /* ── commands ──────────────────────────────────────────── */

  const commands: CommandsAPI = {
    register(def: CommandDefinition) {
      if (!def || typeof def.id !== 'string' || !def.id) throw new Error(`[ext:${id}] commands.register requires an id`);
      if (typeof def.run !== 'function') throw new Error(`[ext:${id}] command "${def.id}" requires run()`);
      const run = def.run;
      const when = def.when;
      const guarded: CommandDefinition = {
        ...def,
        run: guard((...args: unknown[]) => run(...args), `command ${def.id}`, undefined),
        ...(typeof when === 'function' ? { when: guard(() => when(), `command ${def.id} when`, false) } : {})
      };
      return collect(kernel.commands.register(id, guarded));
    },
    run: (commandId, ...args) => runKernelCommand(kernel, commandId, args),
    has: (commandId) => kernel.commands.has(commandId),
    list: () => kernel.commands.list()
  };

  /* ── keybindings ───────────────────────────────────────── */

  const keybindings: KeybindingsAPI = {
    bind: (def: KeybindingDefinition) => collect(kernel.bind(id, def)),
    unbind: (key, all) => kernel.unbind(id, key, all),
    list: () => kernel.listBindings(),
    chordOf: (event) => chordOfEvent(event)
  };

  /* ── effects & transitions ─────────────────────────────── */

  const effects: EffectsAPI = {
    register: (def: EffectDefinition) => collect(kernel.registerEffect(id, def)),
    list: () => kernel.effects.list(),
    get: (effectId) => kernel.effects.get(effectId)
  };

  const transitions: TransitionsAPI = {
    register: (def: TransitionDefinition) => collect(kernel.registerTransition(id, def)),
    list: () => kernel.transitions.list(),
    get: (transitionId) => kernel.transitions.get(transitionId)
  };

  const layers: ExtensionLayersAPI = {
    register: (def: ExtensionLayerDefinition) => collect(kernel.registerLayerType(id, def)),
    list: () => kernel.layerTypes.list(),
    get: (definitionId) => kernel.layerTypes.get(definitionId)
  };

  /* ── theme ─────────────────────────────────────────────── */

  const theme: ThemeAPI = {
    register: (def: ThemeDefinition) => {
      if (!def || typeof def.id !== 'string' || !def.id) throw new Error(`[ext:${id}] theme.register requires an id`);
      for (const key of Object.keys(def.tokens ?? {})) {
        if (!key.startsWith('--')) throw new Error(`[ext:${id}] theme "${def.id}" token "${key}" must start with --`);
      }
      return collect(kernel.themes.register(id, def));
    },
    activate: (themeId) => kernel.activateTheme(themeId),
    active: () => kernel.theme.activeId,
    list: () => kernel.themes.list(),
    scheme: () => kernel.theme.scheme,
    setScheme: (mode) => kernel.setScheme(mode)
  };

  /* ── palette, menus, status ────────────────────────────── */

  const palette: PaletteAPI = {
    registerProvider(provider: PaletteProvider) {
      if (typeof provider !== 'function') throw new Error(`[ext:${id}] palette.registerProvider requires a function`);
      const guarded = guard((query: string): PaletteEntry[] => provider(query) ?? [], 'palette provider', [] as PaletteEntry[]);
      return collect(kernel.registerPaletteProvider(id, guarded));
    },
    open: (query) => deps.paletteOpen(query)
  };

  const menus: MenusAPI = {
    contribute(location: MenuLocation, items: (ctx: Record<string, unknown>) => MenuContribution[]) {
      if (typeof items !== 'function') throw new Error(`[ext:${id}] menus.contribute requires a function`);
      const guarded = guard((ctx: Record<string, unknown>): MenuContribution[] => items(ctx) ?? [], `menu ${location}`, [] as MenuContribution[]);
      return collect(kernel.contributeMenu(id, location, guarded));
    },
    collect: (location, ctx) => kernel.collectMenu(location, ctx)
  };

  const status: StatusAPI = {
    register(item: StatusItem) {
      if (!item || typeof item.id !== 'string' || !item.id) throw new Error(`[ext:${id}] status.register requires an id`);
      if (typeof item.text !== 'function') throw new Error(`[ext:${id}] status item "${item.id}" requires text()`);
      const text = item.text;
      const onClick = item.onClick;
      const guarded: StatusItem = {
        ...item,
        text: guard((): string | null => text(), `status ${item.id}`, null),
        ...(typeof onClick === 'function' ? { onClick: guard(() => onClick(), `status ${item.id} click`, undefined) } : {})
      };
      return collect(kernel.status.register(id, guarded));
    },
    list: () => kernel.status.list()
  };

  /* ── project (origin forced) ───────────────────────────── */

  const project: ProjectAPI = {
    ...deps.project,
    apply: (input: EditCommand | EditCommand[], meta?: Omit<EditMeta, 'origin'>): EditResult =>
      deps.project.apply(input, { ...(meta ?? {}), origin: `ext:${id}` } as Omit<EditMeta, 'origin'>)
  };

  /* ── events ────────────────────────────────────────────── */

  const events: EventsAPI = {
    on(event, fn) {
      const guarded = guard((payload: never) => (fn as (value: never) => void)(payload), `event ${String(event)}`, undefined);
      return collect(kernel.events.on(event, guarded as never, id));
    },
    emit: (event, payload) => kernel.events.emit(event, payload)
  };

  const services: ServicesAPI = {
    register: <T,>(name: string, implementation: T) => collect(kernel.services.register(name, implementation)),
    get: <T,>(name: string) => kernel.services.get<T>(name)
  };

  const api: PowermoveAPI = {
    id,
    apiVersion: 1,
    manifest,
    panels,
    commands,
    keybindings,
    effects,
    transitions,
    layers,
    assets: deps.assets,
    theme,
    palette,
    menus,
    status,
    project,
    anim: deps.anim as AnimAPI,
    model: deps.model as ModelAPI,
    selection: deps.selection as SelectionAPI,
    groups: deps.groups as GroupsAPI,
    transport: deps.transport as TransportAPI,
    history: deps.history as HistoryAPI,
    edit: deps.edit as EditAPI,
    media: {
      ...deps.media as MediaAPI,
      get fonts() { return deps.media?.fonts as MediaAPI['fonts']; },
      registerImportDefaults(defaults) {
        return collect(kernel.services.register(IMPORT_DEFAULTS_SERVICE, validatedImportDefaults(defaults)));
      },
      getImportDefaults() {
        const defaults = kernel.services.get<import('./api').ImportDefaults>(IMPORT_DEFAULTS_SERVICE);
        return defaults ? validatedImportDefaults(defaults) : null;
      }
    },
    inspector: {
      registerSection(section) {
        if (!section || typeof section.id !== 'string' || !section.id || typeof section.title !== 'string'
          || typeof section.build !== 'function' || (section.when !== undefined && typeof section.when !== 'function')
          || (section.after !== undefined && !['content', 'transform', 'effects'].includes(section.after))) {
          throw new Error('inspector.registerSection requires id, title, build and a valid placement.');
        }
        const build = section.build;
        const when = section.when;
        const mounts = new Set<Disposable>();
        const registration = kernel.inspectorSections.register(id, {
          ...section,
          after: section.after ?? 'transform',
          ...(when ? { when: guard(when, `inspector ${section.id} visibility`, false) } : {}),
          build: guard((target, context) => {
            let cleanup: ReturnType<typeof build>;
            try { cleanup = build(target, context); }
            catch (error) { target.replaceChildren(); throw error; }
            const release = guard(() => {
              try {
                if (typeof cleanup === 'function') cleanup();
                else cleanup?.dispose();
              } finally { target.replaceChildren(); }
            }, 'onDispose handler', undefined);
            const mount = { dispose() {
              if (mounts.delete(mount)) release();
            } };
            mounts.add(mount);
            return mount;
          }, `inspector ${section.id}`, undefined)
        });
        return collect({ dispose() {
          registration.dispose();
          for (const mount of [...mounts]) mount.dispose();
        } });
      },
      sections: () => kernel.inspectorSections.list()
    },
    render: deps.render as RenderAPI,
    uiState: deps.uiState as UIStateAPI,
    ui: extensionUI,
    dnd: deps.dnd as DndAPI,
    workspace: deps.workspace as WorkspaceAPI,
    util: deps.util as UtilAPI,
    ease: deps.ease as EaseAPI,
    space3d: deps.space3d as Space3DAPI,
    services,
    storage: deps.storage(id),
    events,
    extensions,
    vars: createVarsAPI(vars),
    host: {
      pm: deps.pm,
      state: deps.state,
      mount: mountComponent,
      haptic: HOST_HAPTIC
    },
    on: events.on,
    log,
    onDispose(fn: () => void) {
      if (typeof fn !== 'function') return;
      collect({ dispose: guard(fn, 'onDispose handler', undefined) });
    }
  };

  return {
    id,
    api,
    setActivating(on: boolean) { activating = on; },
    registerFramePanel,
    disposeAll() {
      if (disposed) return;
      disposed = true;
      // LIFO: later registrations unwind before the state they were built on.
      for (const release of disposers.splice(0).reverse()) {
        try {
          release();
        } catch (error) {
          log('error', 'disposer failed', error);
        }
      }
      kernel.disposeOwner(id);
    }
  };
}
