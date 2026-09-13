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
  AnimAPI,
  AssetRecord,
  AssetsAPI,
  ControlsAPI,
  DndAPI,
  Disposable,
  EaseAPI,
  EditCommand,
  EditAPI,
  EditMeta,
  EditResult,
  ExtensionRecord,
  ExtensionsAPI,
  GroupsAPI,
  HistoryAPI,
  MediaAPI,
  MenuContribution,
  ModelAPI,
  PowermoveAPI,
  Project,
  ProjectAPI,
  RenderAPI,
  Selection,
  SelectionAPI,
  Space3DAPI,
  StorageAPI,
  TransportAPI,
  UIAPI,
  UIStateAPI,
  UtilAPI,
  WorkspaceAPI
} from './api';
import type { ExtensionsBridge } from '../../../shared/extensions';
import { createExtensionAPI, type ExtensionHandle, type HostDeps, type PanelsBackend } from './host';
import { createLoader, type BuiltinFactory, type Loader } from './loader';
import { createKernel, runKernelCommand, type Kernel } from './registries';
import { records as storeRecords } from './extensions.svelte';
import { installRuntimeGlobals } from './runtime-globals';
import { installKernelSignals } from './signals.svelte';
import { DEFAULT_THEME, installThemeApply, type ThemeApplyHandle } from './theme-apply';
import NumField from '../controls/NumField.svelte';
import ColorField from '../controls/ColorField.svelte';
import FillField from '../controls/FillField.svelte';
import FontField from '../controls/FontField.svelte';
import SelectField from '../controls/SelectField.svelte';
import TextField from '../controls/TextField.svelte';
import ToggleField from '../controls/ToggleField.svelte';
import Row from '../controls/Row.svelte';
import Section from '../controls/Section.svelte';
import { channelBinding, compositionBinding, contentBinding, layerFieldBinding, type ControlBindingAPI } from '../controls/binding';
import { doc } from '../state/document.svelte';
import { sel } from '../state/selection.svelte';
import { perf, transport } from '../state/transport.svelte';
import { EditGesture, type EditBinding } from '../controls/gesture';
import {
  CHANNELS_3D,
  inversePlane,
  is3DLayer,
  local3D,
  parent3D,
  perspectiveAmount,
  planeContains,
  planeMatrix,
  projectPoint,
  world3D
} from '../legacy/core/space-3d';
import {
  ASSET_DRAG_MIME,
  FX_DRAG_MIME,
  applyFxDrop,
  hasAssetDrag,
  hasFileDrag,
  hasFxDrag,
  hasMediaDrag,
  readAssetDrag,
  readFxDrag,
  writeAssetDrag
} from '../fx/drop';
import {
  findPanel,
  hasPanel,
  hidePanel,
  insertPanel,
  removePanel,
  restorePanel,
  type Workspace as LayoutWorkspace
} from '../layout/model';

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

const controls: ControlsAPI = {
  NumField: NumField as unknown as ControlsAPI['NumField'],
  ColorField: ColorField as unknown as ControlsAPI['ColorField'],
  FillField: FillField as unknown as ControlsAPI['FillField'],
  FontField: FontField as unknown as ControlsAPI['FontField'],
  SelectField: SelectField as unknown as ControlsAPI['SelectField'],
  TextField: TextField as unknown as ControlsAPI['TextField'],
  ToggleField: ToggleField as unknown as ControlsAPI['ToggleField'],
  Row: Row as unknown as ControlsAPI['Row'],
  Section: Section as unknown as ControlsAPI['Section'],
  binding: null as unknown as ControlsAPI['binding']
};

/** Binding helpers close over typed kernel capabilities. */
function boundControls(api: ControlBindingAPI): ControlsAPI {
  return {
    ...controls,
    binding: {
      channelBinding: (layerId, channel, options) => channelBinding(api, layerId, channel, options),
      layerFieldBinding: (layerId, field, options) => layerFieldBinding(api, layerId, field as any, options),
      contentBinding: (layerId, field, options) => contentBinding(api, layerId, field, options),
      compositionBinding: (field, options) => compositionBinding(api, field as any, options)
    }
  };
}

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

const EMPTY_AFFINE = [1, 0, 0, 1, 0, 0] as const;
const EMPTY_MAT3 = [1, 0, 0, 0, 1, 0, 0, 0, 1] as const;
const EMPTY_MAT4 = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1] as const;

function makeAnim(PM: LegacyPM): AnimAPI {
  const emptyErrors = new WeakMap<object, string>();
  return {
    ev: (layer, key, time) => PM?.ev?.(layer, key, time) ?? null,
    evP: (layer, prop, time, key) => PM?.evP?.(layer, prop, time, key) ?? null,
    active: (layer, time) => !!PM?.active?.(layer, time),
    findProp: (layer, key) => PM?.findProp?.(layer, key) ?? null,
    allProps: (layer) => PM?.allProps?.(layer) ?? [],
    hasKeyAt: (layer, prop, time) => PM?.hasKeyAt?.(layer, prop, time) ?? null,
    setKey: (...args) => PM?.setKey?.(...args) ?? null,
    setKeyOn: (...args) => PM?.setKeyOn?.(...args) ?? null,
    removeKey: (prop, key) => PM?.removeKey?.(prop, key),
    applyEaseTo: (keys, name) => PM?.applyEaseTo?.(keys, name),
    wouldCycle: (layer, parentId) => !!PM?.wouldCycle?.(layer, parentId),
    resolveContent: (...args) => PM?.resolveContent?.(...args) ?? args[0].d,
    get expressionErrors() { return PM?.expressionErrors ?? emptyErrors; },
    version: () => Number(PM?.animVersion?.() ?? 0),
    touch: () => PM?.touch?.(),
    worldMatrix: (layer, time) => PM?.worldMatrix?.(layer, time) ?? [...EMPTY_AFFINE],
    localMatrix: (layer, time) => PM?.localMatrix?.(layer, time) ?? [...EMPTY_AFFINE],
    transformParentMatrix: (...args) => PM?.transformParentMatrix?.(...args) ?? [...EMPTY_AFFINE],
    mul: (left, right) => PM?.mul?.(left, right) ?? [...EMPTY_AFFINE]
  } as AnimAPI;
}

function makeModel(PM: LegacyPM): ModelAPI {
  return {
    P: ((...args: [unknown, object?]) => PM?.P?.(...args) ?? { v: args[0], kf: [], expr: null, ...(args[1] ?? {}) }) as ModelAPI['P'],
    get CH() { return PM?.CH ?? {}; },
    KF: ((...args: [number, unknown, string?]) => PM?.KF?.(...args)) as ModelAPI['KF'],
    get BLENDS() { return PM?.BLENDS ?? []; },
    get TYPE_META() { return PM?.TYPE_META ?? {}; },
    get MASK_SHAPES() { return PM?.MASK_SHAPES ?? []; },
    mkLayer: (...args) => PM?.mkLayer?.(...args),
    mkMask: (...args) => PM?.mkMask?.(...args),
    mkProject: (...args) => PM?.mkProject?.(...args),
    cloneLayer: (layer) => PM?.cloneLayer?.(layer),
    normalizeFill: (value, fallback) => PM?.normalizeFill?.(value, fallback),
    layerDefinition: (id) => PM?.layerDefinition?.(id),
    curComp: () => PM?.curComp?.() ?? PM?.proj,
    layer: (id) => PM?.L?.(id) ?? null,
    byName: (name) => PM?.byName?.(name) ?? null
  } as ModelAPI;
}

function makeSelection(PM: LegacyPM): SelectionAPI {
  const empty: Selection = { layers: [], keys: [], chan: null };
  const current = (): Selection => PM?.sel ?? empty;
  return {
    get: current,
    layers: () => current().layers,
    first: () => PM?.firstSel?.() ?? null,
    keys: () => current().keys,
    chan: () => current().chan,
    set: (partial) => {
      if (!PM?.sel) return;
      const before = { ...PM.sel, layers: [...(PM.sel.layers ?? [])], keys: [...(PM.sel.keys ?? [])] };
      Object.assign(PM.sel, partial);
      PM?.hist?.selection?.(before, PM.sel);
      PM?.bus?.emit?.('sel');
      PM?.invalidate?.();
    },
    select: (ids, add) => PM?.selectLayers?.(ids, add),
    resolveSelectedKeys: () => PM?.resolveSelectedKeys?.() ?? [],
    get keySelectionActive() { return !!PM?.TL?.keySelectionActive; },
    set keySelectionActive(value: boolean) { if (PM?.TL) PM.TL.keySelectionActive = value; }
  };
}

function makeGroups(PM: LegacyPM): GroupsAPI {
  return {
    ancestors: (...args) => PM?.groupAncestors?.(...args) ?? [],
    transformRoots: (ids) => PM?.transformRoots?.(ids) ?? [],
    span: (group) => PM?.groupSpan?.(group) ?? { from: 0, dur: 0 },
    expand: (ids) => PM?.expandGroups?.(ids) ?? [...ids],
    normalizeStack: () => PM?.normalizeStack?.(),
    moveToGroup: (ids, target) => PM?.moveToGroup?.(ids, target) ?? { ids: [...ids], group: target }
  };
}

function makeTransport(PM: LegacyPM): TransportAPI {
  const emptyPerf = { fps: 0, ms: 0, drops: 0, budget: 0, auto: false };
  return {
    time: () => Number(PM?.time ?? 0),
    setTime: (...args) => PM?.setTime?.(...args),
    play: () => PM?.play?.(),
    pause: () => PM?.pause?.(),
    toggle: () => PM?.toggle?.(),
    playing: () => !!PM?.playing,
    step: (frames) => PM?.step?.(frames),
    get quality() { return Number(PM?.quality ?? 0); },
    set quality(value: number) { PM.quality = value; },
    get perf() { return PM?.perf ?? emptyPerf; },
    invalidate: (...args) => PM?.invalidate?.(...args),
    get previewResolution() { return PM?.previewResolution ?? null; },
    set previewResolution(value: string | number | null) { PM.previewResolution = value; }
  };
}

function makeHistory(PM: LegacyPM): HistoryAPI {
  return {
    do: ((label: string, fn: () => unknown) => PM?.hist?.do?.(label, fn)) as HistoryAPI['do'],
    begin: (...args) => PM?.hist?.begin?.(...args),
    commit: (...args) => !!PM?.hist?.commit?.(...args),
    cancel: () => PM?.hist?.cancel?.(),
    undo: () => !!PM?.hist?.undo?.(),
    redo: () => !!PM?.hist?.redo?.(),
    external: (...args) => PM?.hist?.external?.(...args) ?? null,
    selection: (before, after) => PM?.hist?.selection?.(before, after) ?? null
  };
}

function makeEdit(PM: LegacyPM): EditAPI {
  const unavailable = (): EditResult => ({ ok: false, message: 'editing engine unavailable' });
  return {
    apply: (...args) => PM?.Edit?.apply?.(...args) ?? unavailable(),
    begin: (...args) => PM?.Edit?.begin?.(...args),
    commit: (...args) => PM?.Edit?.commit?.(...args),
    cancel: () => !!PM?.Edit?.cancel?.(),
    dispatch: (command) => PM?.Edit?.dispatch?.(command) ?? unavailable(),
    mutate: ((...args: [string, () => unknown, EditMeta?]) => PM?.Edit?.mutate?.(...args) ?? unavailable()) as EditAPI['mutate']
  };
}

function makeMedia(PM: LegacyPM): MediaAPI {
  const emptyFonts = {
    bundled: [], system: [], families: [],
    setSystemFamilies: (_values: string[]) => {},
    options: (current: string) => current ? [current] : [],
    ensure: async (_family: string, _weight?: number) => {}
  };
  return {
    timing: {
      isTimed: (layer) => !!PM?.MediaTiming?.isTimed?.(layer),
      rate: (layer) => Number(PM?.MediaTiming?.rate?.(layer) ?? 1),
      earliestStart: (layer) => Number(PM?.MediaTiming?.earliestStart?.(layer) ?? layer.from ?? 0)
    },
    importFiles: (...args) => PM?.importFiles?.(...args) ?? Promise.resolve(undefined),
    commandForAsset: (...args) => PM?.commandForAsset?.(...args),
    audio: {
      drawWaveform: (...args) => !!PM?.Audio?.drawWaveform?.(...args)
    },
    assets: {
      get: (id) => PM?.assets?.get?.(id),
      add: (...args) => PM?.assets?.add?.(...args) ?? Promise.resolve(undefined),
      kind: (file) => PM?.assetKind?.(file) ?? null
    },
    get fonts() { return PM?.Fonts ?? emptyFonts; }
  } as MediaAPI;
}

function makeRender(PM: LegacyPM): RenderAPI {
  return {
    gl: {
      bounds: (layer, time) => PM?.GL?.bounds?.(layer, time) ?? null,
      pick: (x, y, time, options) => PM?.GL?.pick?.(x, y, time, options) ?? null,
      init: (...args) => !!PM?.GL?.init?.(...args),
      resize: (...args) => !!PM?.GL?.resize?.(...args),
      compileError: (key) => PM?.GL?.compileError?.(key) ?? null,
      get previewViewport() { return PM?.GL?.previewViewport ?? null; },
      get context() { return PM?.GL?.gl ?? null; }
    },
    raster: (...args) => PM?.raster?.(...args) ?? null,
    renderFrameTo: (...args) => PM?.renderFrameTo?.(...args),
    snapshot: (...args) => PM?.Export?.snapshot?.(...args) ?? ''
  } as RenderAPI;
}

function makeUIState(PM: LegacyPM): UIStateAPI {
  return {
    getLayerCollapsed: (layer) => !!PM?.UIState?.getLayerCollapsed?.(layer),
    setLayerCollapsed: (layer, collapsed) => !!PM?.UIState?.setLayerCollapsed?.(layer, collapsed),
    getKeyHandles: (key) => PM?.UIState?.getKeyHandles?.(key) ?? null,
    setKeyHandles: (key, patch) => PM?.UIState?.setKeyHandles?.(key, patch) ?? null,
    getFxOpen: (effect) => !!PM?.UIState?.getFxOpen?.(effect),
    setFxOpen: (effect, open) => !!PM?.UIState?.setFxOpen?.(effect, open),
    getReveal: (layer) => PM?.UIState?.getReveal?.(layer) ?? null,
    setReveal: (layer, keys) => PM?.UIState?.setReveal?.(layer, keys) ?? null,
    getShaderMeta: (layer) => PM?.UIState?.getShaderMeta?.(layer) ?? null,
    setShaderMeta: (layer, patch) => PM?.UIState?.setShaderMeta?.(layer, patch) ?? null
  };
}

function makeDnd(PM: LegacyPM): DndAPI {
  return {
    ASSET_MIME: ASSET_DRAG_MIME,
    FX_MIME: FX_DRAG_MIME,
    startAssetDrag: writeAssetDrag,
    get mediaDrag() { return PM?.mediaDrag ?? null; },
    set mediaDrag(value) { PM.mediaDrag = value; },
    hasAssetDrag,
    hasFileDrag,
    hasMediaDrag,
    readAssetDrag,
    hasFxDrag,
    readFxDrag,
    applyFxDrop: (payload, layerId, edge) => applyFxDrop(payload, layerId, edge, PM)
  };
}

function makeWorkspace(PM: LegacyPM): WorkspaceAPI {
  const current = () => PM?.WS?.current ?? null;
  const mutate = (...args: [(workspace: LayoutWorkspace) => void, { inPlace?: boolean }?]): unknown =>
    PM?.WS?.mutate?.(...args);
  return {
    current,
    mutate: ((...args: Parameters<WorkspaceAPI['mutate']>) => mutate(
      args[0] as unknown as (workspace: LayoutWorkspace) => void,
      ...args.slice(1) as [{ inPlace?: boolean }?]
    ) as ReturnType<WorkspaceAPI['current']>) as WorkspaceAPI['mutate'],
    hasPanel: (id) => {
      const workspace = current();
      return !!workspace && hasPanel(workspace as unknown as LayoutWorkspace, id);
    },
    addPanel: (id, dock = 'right', index) => {
      mutate((workspace) => {
        removePanel(workspace, id);
        workspace.hiddenPanels = (workspace.hiddenPanels ?? []).filter((item) => item.id !== id);
        insertPanel(workspace, { id }, dock, index);
      });
    },
    movePanel: (id, dock, index) => {
      let moved = false;
      mutate((workspace) => {
        const found = findPanel(workspace, id);
        if (!found) return;
        const from = found.dock.panels.indexOf(found.spec);
        if (found.dock.id === dock && from === index) return;
        const spec = { ...found.spec };
        removePanel(workspace, id);
        insertPanel(workspace, spec, dock, index);
        workspace.hiddenPanels = (workspace.hiddenPanels ?? []).filter((item) => item.id !== id);
        moved = true;
      });
      return moved;
    },
    removePanel: (id) => void mutate((workspace) => removePanel(workspace, id)),
    hidePanel: (id) => {
      let hidden = false;
      mutate((workspace) => { hidden = hidePanel(workspace, id); });
      return hidden;
    },
    restorePanel: (id) => {
      let restored = false;
      mutate((workspace) => { restored = restorePanel(workspace, id); });
      return restored;
    },
    refresh: (id) => PM?.Layout?.refresh?.(id)
  } as WorkspaceAPI;
}

function makeUtil(PM: LegacyPM): UtilAPI {
  return {
    round: (...args) => Number(PM?.round?.(...args) ?? Number(args[0].toFixed(args[1] ?? 2))),
    clamp: (value, min, max) => Number(PM?.clamp?.(value, min, max) ?? Math.max(min, Math.min(max, value))),
    lerp: (from, to, amount) => Number(PM?.lerp?.(from, to, amount) ?? from + (to - from) * amount),
    snapF: (time, fps) => Number(PM?.snapF?.(time, fps) ?? Math.round(time * fps) / fps),
    tc: (...args) => String(PM?.tc?.(...args) ?? args[0]),
    parseTc: (...args) => PM?.parseTc?.(...args) ?? null,
    uid: (...args) => String(PM?.uid?.(...args) ?? `${args[0] ?? 'l'}-${Math.random().toString(36).slice(2)}`),
    hex2rgb: (hex) => PM?.hex2rgb?.(hex) ?? [],
    rgb2hex: (red, green, blue) => String(PM?.rgb2hex?.(red, green, blue) ?? '')
  };
}

function makeEase(PM: LegacyPM): EaseAPI {
  return {
    nameOf: (easeOut, easeIn) => String(PM?.Ease?.nameOf?.(easeOut, easeIn) ?? 'custom'),
    get PRESETS() { return PM?.Ease?.PRESETS ?? {}; }
  };
}

function makeSpace3D(PM: LegacyPM): Space3DAPI {
  const attempt = <T,>(fn: () => T, fallback: T): T => {
    try { return fn(); } catch { return fallback; }
  };
  return {
    CHANNELS_3D,
    local3D: (layer, time) => attempt(() => local3D(PM, layer, time), [...EMPTY_MAT4]),
    parent3D: (...args) => attempt(() => parent3D(PM, ...args), [...EMPTY_MAT4]),
    world3D: (...args) => attempt(() => world3D(PM, ...args), [...EMPTY_MAT4]),
    is3DLayer: (layer) => attempt(() => is3DLayer(PM, layer), false),
    perspectiveAmount: (layer, time) => attempt(() => perspectiveAmount(PM, layer, time), 0.001),
    planeMatrix: (...args) => attempt(() => planeMatrix(PM, ...args), [...EMPTY_MAT3]),
    projectPoint,
    inversePlane,
    planeContains: (layer, time, x, y, bounds) => attempt(() => planeContains(PM, layer, time, x, y, bounds), false)
  } as Space3DAPI;
}

function makeUI(
  PM: LegacyPM,
  controlAPI: ControlBindingAPI,
  gestureAPI: Pick<PowermoveAPI, 'edit' | 'history'>
): UIAPI {
  class BoundEditGesture extends EditGesture {
    constructor(binding: EditBinding) { super(gestureAPI, binding); }
  }
  return {
    controls: boundControls(controlAPI),
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
    },
    drag: (event, options) => PM?.drag?.(event, options) ?? { cancel: () => {} },
    closeMenus: () => PM?.closeMenus?.(),
    showLayerMenu: (...args) => PM?.showLayerMenu?.(...args),
    showParentMenu: (ids, event) => PM?.showParentMenu?.(ids, event),
    beginParentPick: (event, ids) => PM?.beginParentPick?.(event, ids),
    openShaderEditor: (...args) => PM?.openShaderEditor?.(...args),
    gesture: BoundEditGesture
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

function makeAssets(PM: LegacyPM): AssetsAPI {
  return {
    pick: (options = {}) => new Promise<File[]>((resolve, reject) => {
      const input = window.document.createElement('input');
      input.type = 'file';
      input.accept = options.accept ?? '';
      input.multiple = options.multiple === true;
      input.style.cssText = 'position:fixed;width:1px;height:1px;opacity:0;pointer-events:none';
      let settled = false;
      const finish = (files: File[]) => {
        if (settled) return;
        settled = true;
        input.remove();
        resolve(files);
      };
      input.addEventListener('change', () => finish(Array.from(input.files ?? [])), { once: true });
      input.addEventListener('cancel', () => finish([]), { once: true });
      window.document.body.appendChild(input);
      try { input.click(); }
      catch (error) { input.remove(); reject(error); }
    }),
    import: async (file, options = {}) => {
      if (!(file instanceof File)) throw new Error('assets.import requires a File');
      const live = await PM?.assets?.add?.(file, { layerDefinition: options.layerDefinition });
      if (!live?.id) throw new Error(`Could not import ${file.name || 'asset'}`);
      return (PM?.proj?.assets?.[live.id] ?? {
        id: live.id, name: live.name, kind: live.kind, size: live.size
      }) as AssetRecord;
    },
    get: (id) => PM?.proj?.assets?.[id] as AssetRecord | undefined,
    readText: async (id) => {
      const meta = PM?.proj?.assets?.[id];
      if (!meta) throw new Error(`Asset not found: ${id}`);
      const live = PM?.assets?.get?.(id);
      if (typeof live?.sourceText === 'string') return live.sourceText;
      const blob = live?.blob instanceof Blob ? live.blob : await PM?.MediaStore?.get?.(meta);
      if (!(blob instanceof Blob)) throw new Error(`Asset data is missing: ${meta.name || id}`);
      if (blob.size > 64 * 1024 * 1024) throw new Error('Text asset is larger than 64 MB');
      return blob.text();
    }
  };
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
    open: (id, placement) => {
      const explicitOptions = typeof placement === 'object' && placement !== null;
      const options = typeof placement === 'string' ? { dock: placement } : (placement ?? {});
      const dock = options.dock;
      if (!PM?.Layout?.hasPanel?.(current(), id)) {
        PM?.WS?.mutate?.((workspace: any) => {
          /* A hidden panel remembers its dock, index and size — restore beats
             a fresh add, which would drop all three. */
          if (PM?.Layout?.restorePanel?.(workspace, id)) {
            if (explicitOptions && workspace?.layout?.docks && (dock != null || options.index != null)) {
              const found = findPanel(workspace as LayoutWorkspace, id);
              if (found) {
                const spec = { ...found.spec };
                const targetDock = dock ?? found.dock.id;
                const targetIndex = options.index ?? found.dock.panels.indexOf(found.spec);
                removePanel(workspace as LayoutWorkspace, id);
                insertPanel(workspace as LayoutWorkspace, spec, targetDock, targetIndex);
              }
            }
            return;
          }
          if (explicitOptions && workspace?.layout?.docks) {
            workspace.hiddenPanels = (workspace.hiddenPanels ?? []).filter((item: { id?: string }) => item.id !== id);
            insertPanel(workspace as LayoutWorkspace, { id }, dock ?? 'center', options.index);
          } else PM?.Layout?.addPanel?.(workspace, id, dock ?? 'center');
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

  const project = makeProject(PM);
  const anim = makeAnim(PM);
  const model = makeModel(PM);
  const transportAPI = makeTransport(PM);
  const history = makeHistory(PM);
  const edit = makeEdit(PM);
  const util = makeUtil(PM);
  const controlAPI: ControlBindingAPI = { project, anim, model, transport: transportAPI, util };

  const deps: Omit<HostDeps, 'reportRuntimeError'> = {
    pm: PM,
    state: { doc, sel, transport, perf },
    ui: makeUI(PM, controlAPI, { edit, history }),
    project,
    anim,
    model,
    selection: makeSelection(PM),
    groups: makeGroups(PM),
    transport: transportAPI,
    history,
    edit,
    media: makeMedia(PM),
    render: makeRender(PM),
    uiState: makeUIState(PM),
    dnd: makeDnd(PM),
    workspace: makeWorkspace(PM),
    util,
    ease: makeEase(PM),
    space3d: makeSpace3D(PM),
    assets: makeAssets(PM),
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

  /* Structured extension layers keep only definition ids and editable values
     in the project. The live renderer definition comes from the registry, so
     disable/reload can never erase project data. */
  PM.layerDefinition = (id: string) => kernel.layerTypes.get(id);
  const layerTypesChanged = kernel.layerTypes.onChange((change) => {
    const definition = kernel.layerTypes.get(change.id);
    if (definition) {
      const containers = [PM?.proj, ...Object.values(PM?.proj?.comps ?? {})] as Array<Record<string, any> | undefined>;
      for (const container of containers) for (const layer of container?.layers ?? []) {
        if (layer?.type !== 'extension' || layer?.d?.definition !== change.id) continue;
        layer.d.params = layer.d.params && typeof layer.d.params === 'object' ? layer.d.params : {};
        for (const param of definition.params) {
          if (!layer.d.params[param.k] || typeof layer.d.params[param.k] !== 'object') {
            layer.d.params[param.k] = PM?.P?.(param.def) ?? { v: param.def, kf: [], expr: null };
          }
        }
      }
      PM?.ProjectIndex?.invalidateKeyframes?.();
    }
    PM?.GL?.dropPrograms?.(`extension:${change.id}:`);
    PM?.bus?.emit?.('layers');
    PM?.invalidate?.('render');
    PM?.Inspector?.refresh?.();
  });
  subscriptions.push(() => layerTypesChanged.dispose());

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
  on('fonts', (families: string[]) => kernel.events.emit('fonts', families));
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
  const refreshLayout = (): void => {
    const pm = installed.deps.pm as Record<string, any>;
    const workspace = pm?.Layout?.ws ?? pm?.WS?.current;
    if (workspace && typeof pm?.Layout?.apply === 'function') pm.Layout.apply(workspace);
    else pm?.bus?.emit?.('layout');
  };
  const booting = loader.boot();
  await loader.builtinsReady;
  /* Built-ins have now registered their themes, so the persisted id resolves. */
  installed.restoreTheme();
  /* ToolbarMount retries its lazy panel lookup on layout. It mounted before
     extensions booted. Reapply the current workspace so panel slots that were
     empty while their built-ins were unavailable get another mount pass. */
  refreshLayout();
  await booting;
  /* Extensions may add or replace layout contributions during full boot. */
  refreshLayout();
  return loader;
}

export type { Disposable };
