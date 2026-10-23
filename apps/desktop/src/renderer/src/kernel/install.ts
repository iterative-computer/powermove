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
  Layer,
  RasterWindow,
  PreviewViewport,
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
import type { VarsBridge } from '../../../shared/vars-ipc';
import type { PowermoveExtensionsBridge } from '../../../shared/ipc';
import { createExtensionAPI, type ExtensionHandle, type HostDeps, type PanelsBackend } from './host';
import { createLoader, type BuiltinFactory, type Loader, type LoaderOptions } from './loader';
import { createKernel, runKernelCommand, type Kernel } from './registries';
import { makeScene3DAPI } from '../core/scene3d/api';
import { engineRaster, engineRasterPrepared, engineRasterSync, locateLayer, type RasterCrop, type RasterHost } from '../engine-connection/raster-surface';
import { records as storeRecords } from './extensions.svelte';
import { installRuntimeGlobals } from './runtime-globals';
import { installKernelSignals } from './signals.svelte';
import { DEFAULT_THEME, installThemeApply, type ThemeApplyHandle } from './theme-apply';
import NumField from '../controls/NumField.svelte';
import ColorField from '../controls/ColorField.svelte';
import FillField from '../controls/FillField.svelte';
import RampField from '../controls/RampField.svelte';
import PointField from '../controls/PointField.svelte';
import SliderField from '../controls/SliderField.svelte';
import Disclosure from '../controls/Disclosure.svelte';
import FontField from '../controls/FontField.svelte';
import SelectField from '../controls/SelectField.svelte';
import TextField from '../controls/TextField.svelte';
import ToggleField from '../controls/ToggleField.svelte';
import Row from '../controls/Row.svelte';
import Section from '../controls/Section.svelte';
import Segmented from '../controls/Segmented.svelte';
import { channelBinding, compositionBinding, contentBinding, layerFieldBinding, type ControlBindingAPI } from '../controls/binding';
import { doc } from '../state/document.svelte';
import { sel } from '../state/selection.svelte';
import { perf, transport } from '../state/transport.svelte';
import { EditGesture, type EditBinding } from '../controls/gesture';
import { confirm as confirmPrompt } from '../overlays/confirm';
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
import { bridge as hostBridge } from './bridge';
import { parseExtensionUrl } from '../../../shared/extension-url';
import { fetchRemoteMedia, type AdmitDownload, type ImportUrl } from './remote-media';
import { touchedDocument, writeDocument } from '../engine-doc/write';
import { assetAudio, assetSource } from './live-media';
import { createRasterCompat, TICKS_PER_SECOND } from './raster-compat';
import { compatContext, createCompatContextHost } from './compat-context';
import { runCommandWhenReady } from '../engine-doc/readiness';

type LegacyPM = Record<string, any>;
type ExtensionsHostBridge = ExtensionsBridge & Partial<Pick<PowermoveExtensionsBridge, 'fork'>>;

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
  Segmented,
  NumField: NumField as unknown as ControlsAPI['NumField'],
  ColorField: ColorField as unknown as ControlsAPI['ColorField'],
  FillField: FillField as unknown as ControlsAPI['FillField'],
  RampField: RampField as unknown as ControlsAPI['RampField'],
  PointField: PointField as unknown as ControlsAPI['PointField'],
  SliderField: SliderField as unknown as ControlsAPI['SliderField'],
  Disclosure: Disclosure as unknown as ControlsAPI['Disclosure'],
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
    latest: () => PM?.proj as Project | undefined,
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
    snapshot: async (t, maxWidth) => {
      // With the new engine on, the engine draws it (never the current engine).
      // Today's path is the synchronous capture: a Blender composition with
      // no prepared frame rejects with today's error; other failures are ''.
      const engine = engineRender(PM);
      if (engine) {
        return engine.snapshotDataUrl(t ?? PM?.time ?? 0, maxWidth ?? 480, PM.proj).catch((error: { code?: string; message?: string }) => {
          if (error?.code === 'blenderFailed') throw new Error(String(error.message));
          return '';
        });
      }
      return (await PM?.Export?.snapshot?.(t ?? PM?.time ?? 0, maxWidth ?? 480)) ?? '';
    }
  };
}

const EMPTY_AFFINE = [1, 0, 0, 1, 0, 0] as const;
const EMPTY_MAT3 = [1, 0, 0, 0, 1, 0, 0, 0, 1] as const;
const EMPTY_MAT4 = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1] as const;

/** The native evaluator (engine-doc), when the flag is on; it answers through
 *  pm-wasm and falls back to the legacy code itself. */
/** The engine's render transport while "Use New Engine" is on
 *  (engine-connection/render-transport.ts). */
function engineRender(PM: LegacyPM): import('../engine-connection/render-transport').EngineRenderTransport | null {
  return PM?.EngineRender ?? null;
}

function native(PM: LegacyPM): import('../engine-doc/native').NativeApi | null {
  const doc = PM?.EngineDoc;
  return doc?.enabled && doc.api ? doc.api : null;
}

function makeAnim(PM: LegacyPM): AnimAPI {
  const emptyErrors = new WeakMap<object, string>();
  /* With the native evaluator, expression errors land on the live channels
     pm-wasm evaluated; channels only the renderer evaluated keep theirs. */
  class ErrorsView extends WeakMap<object, string> {
    override get(key: object) { const n = native(PM); return n?.errors.has(key) ? n.errors.get(key) : (PM?.expressionErrors ?? emptyErrors).get(key); }
    override has(key: object) { const n = native(PM); return !!n?.errors.has(key) || !!(PM?.expressionErrors ?? emptyErrors).has(key); }
  }
  const errorsView = new ErrorsView();
  return {
    ev: (layer, key, time) => { const n = native(PM); return (n ? n.ev(layer, key, time) : PM?.ev?.(layer, key, time)) ?? null; },
    evP: (layer, prop, time, key) => { const n = native(PM); return (n ? n.evP(layer, prop, time, key) : PM?.evP?.(layer, prop, time, key)) ?? null; },
    active: (layer, time) => { const n = native(PM); return !!(n ? n.active(layer, time) : PM?.active?.(layer, time)); },
    findProp: (layer, key) => { const n = native(PM); return (n ? n.findProp(layer, key) : PM?.findProp?.(layer, key)) ?? null; },
    allProps: (layer) => { const n = native(PM); return (n ? n.allProps(layer) : PM?.allProps?.(layer)) ?? []; },
    hasKeyAt: (layer, prop, time) => { const n = native(PM); return (n ? n.hasKeyAt(layer, prop, time) : PM?.hasKeyAt?.(layer, prop, time)) ?? null; },
    setKey: (...args) => PM?.setKey?.(...args) ?? null,
    setKeyOn: (...args) => PM?.setKeyOn?.(...args) ?? null,
    removeKey: (prop, key) => PM?.removeKey?.(prop, key),
    applyEaseTo: (keys, name) => PM?.applyEaseTo?.(keys, name),
    wouldCycle: (layer, parentId) => { const n = native(PM); return !!(n ? n.wouldCycle(layer, parentId) : PM?.wouldCycle?.(layer, parentId)); },
    resolveContent: (...args) => { const n = native(PM); return (n ? n.resolveContent(...args) : PM?.resolveContent?.(...args)) ?? args[0].d; },
    get expressionErrors() { return native(PM) ? errorsView : PM?.expressionErrors ?? emptyErrors; },
    version: () => Number(PM?.animVersion?.() ?? 0),
    touch: () => PM?.touch?.(),
    worldMatrix: (layer, time) => { const n = native(PM); return (n ? n.worldMatrix(layer, time) : PM?.worldMatrix?.(layer, time)) ?? [...EMPTY_AFFINE]; },
    localMatrix: (layer, time) => { const n = native(PM); return (n ? n.localMatrix(layer, time) : PM?.localMatrix?.(layer, time)) ?? [...EMPTY_AFFINE]; },
    transformParentMatrix: (...args) => { const n = native(PM); return (n ? n.transformParentMatrix(...args) : PM?.transformParentMatrix?.(...args)) ?? [...EMPTY_AFFINE]; },
    mul: (left, right) => { const n = native(PM); return (n ? n.mul(left, right) : PM?.mul?.(left, right)) ?? [...EMPTY_AFFINE]; }
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
    ancestors: (...args) => { const n = native(PM); return (n ? n.groupAncestors(...args) : PM?.groupAncestors?.(...args)) ?? []; },
    transformRoots: (ids) => { const n = native(PM); return (n ? n.transformRoots(ids) : PM?.transformRoots?.(ids)) ?? []; },
    bounds: (group, time) => { const n = native(PM); return (n ? n.groupBounds(group, time) : PM?.groupBounds?.(group, time)) ?? null; },
    span: (group) => { const n = native(PM); return (n ? n.groupSpan(group) : PM?.groupSpan?.(group)) ?? { from: 0, dur: 0 }; },
    expand: (ids) => { const n = native(PM); return (n ? n.expandGroups(ids) : PM?.expandGroups?.(ids)) ?? [...ids]; },
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
  const warnedObject3dAssets = new Set<string>();
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
      earliestStart: (layer) => Number(PM?.MediaTiming?.earliestStart?.(layer) ?? layer.from ?? 0),
      startPatch: (layer, nextFrom) => PM?.MediaTiming?.startPatch?.(layer, nextFrom) ?? null,
      endPatch: (layer, cut) => PM?.MediaTiming?.endPatch?.(layer, cut) ?? null
    },
    importFiles: (...args) => PM?.importFiles?.(...args) ?? Promise.resolve(undefined),
    commandForAsset: (...args) => PM?.commandForAsset?.(...args),
    audio: {
      drawWaveform: (...args) => !!PM?.Audio?.drawWaveform?.(...args)
    },
    assets: {
      get: (id) => {
        const asset = PM?.assets?.get?.(id);
        if (PM?.EngineDoc?.enabled && asset?.object3d && !warnedObject3dAssets.has(id)) {
          warnedObject3dAssets.add(id);
          console.warn('[agent] media.assets.get(id).object3d is deprecated; use scene3d APIs for new extensions. The compatibility Object3D remains available for apiVersion 1–3.');
        }
        return asset;
      },
      add: (...args) => PM?.assets?.add?.(...args) ?? Promise.resolve(undefined),
      kind: (file) => PM?.assetKind?.(file) ?? null,
      // The replacements of the deprecated live fields (live-media.ts).
      source: (id, options) => assetSource(PM, id, options),
      audio: (id, options) => assetAudio(PM, id, options)
    },
    get fonts() { return PM?.Fonts ?? emptyFonts; }
  } as MediaAPI;
}

/* `render.raster()` with "Use New Engine" on: while a public call runs,
   the raster service's base (beneath any extension wrapper) is the engine's
   raster (engine-connection/raster-surface.ts). The compositor's and the
   editing helpers' own `PM.raster` calls keep today's rasterizer. */
let publicRaster = 0;

function rasterHost(PM: LegacyPM): RasterHost {
  const unwrap = (v: any) => PM?.EngineDoc?.tracker?.unwrap?.(v) ?? v;
  return {
    locate: (layer) => locateLayer(PM, layer, unwrap),
    captionBounds: (layer, time) => PM?.captionBounds?.(layer, time) ?? null
  };
}

/** The engine's raster in today's shape, or null (logged) when it fails. */
function engineRasterBase(PM: LegacyPM, engine: NonNullable<ReturnType<typeof engineRender>>, args: Parameters<RenderAPI['raster']>): ReturnType<RenderAPI['raster']> {
  // Today's `PM.raster(L, scale = 1, ...)` uses any other scale as given
  // (zero and negatives too); the transport maps it (`layerRasterScale`).
  const [layer, scale = 1, time, , crop] = args;
  try {
    return engineRasterSync(engine, rasterHost(PM), layer, Number(scale), time ?? PM?.time ?? 0, crop as RasterCrop) as ReturnType<RenderAPI['raster']>;
  } catch (error) {
    console.warn(`[engine] render.raster(${String((layer as any)?.id ?? '')}) failed: ${error instanceof Error ? error.message : String(error)}`);
    return null;
  }
}

/** The canvas the last `render.gl.init` got (one per window, as `GL.init`). */
const initCanvas: { current: HTMLCanvasElement | null } = { current: null };

/** Binds the engine's presentation to the canvas `init` got before the engine connection installed. */
export function bindLate(PM: LegacyPM, canvas: HTMLCanvasElement | null): boolean {
  const binding = PM?.EngineCanvas;
  if (!binding || !canvas || binding.bound || !canvas.isConnected) return false;
  return binding.bind?.(canvas) === true;
}

function makeRender(PM: LegacyPM): RenderAPI {
  /* With the native evaluator a text layer's placement metrics come from
     pm-wasm at scale 1 (compared in dual mode), as they do with the flag
     off; the pixels are the engine's with "Use New Engine" on. */
  const placed = (surface: ReturnType<RenderAPI['raster']>, args: Parameters<RenderAPI['raster']>) => {
    const n = native(PM);
    // A wrapper owns its returned geometry: it may have rasterized a
    // detached layer with different content or deliberately moved its anchor.
    if (!n || !surface || (args[1] ?? 1) !== 1 || PM?.RasterCompat?.wrapped?.()) return surface;
    return n.rasterSurface(surface, args[0], args[2] ?? PM?.time ?? 0);
  };
  const raster = (...args: Parameters<RenderAPI['raster']>) => {
    if (!engineRender(PM)) return placed(PM?.raster?.(...args) ?? null, args);
    publicRaster++;
    try {
      return placed(PM?.raster?.(...args) ?? null, args);
    } finally {
      publicRaster--;
    }
  };
  return {
    gl: {
      bounds: (layer, time) => { const n = native(PM); return (n ? n.bounds(layer, time) : PM?.GL?.bounds?.(layer, time)) ?? null; },
      pick: (x, y, time, options) => { const n = native(PM); return (n ? n.pick(x, y, time, options) : PM?.GL?.pick?.(x, y, time, options)) ?? null; },
      /* With the new engine on, engine presentation is bound to the canvas
         the caller passes (engine-connection/canvas-binding.ts); the
         compositor keeps its context for capability routing, scaffolding
         until it is removed. */
      init: (canvas, options) => {
        const legacy = !!PM?.GL?.init?.(canvas, options);
        initCanvas.current = canvas;
        return PM?.EngineCanvas?.bind?.(canvas) === true || legacy;
      },
      resize: (width, height, viewport) => {
        const legacy = !!PM?.GL?.resize?.(width, height, viewport);
        // A viewer that called `init` before the engine connection was up
        // (frozen viewers call it once, at mount) is bound now.
        bindLate(PM, initCanvas.current);
        PM?.EngineCanvas?.resize?.(width, height, viewport);
        return legacy || !!PM?.EngineCanvas?.bound;
      },
      compileError: (key) => PM?.GL?.compileError?.(key) ?? null,
      get previewViewport() { return PM?.GL?.previewViewport ?? null; },
      /* The canvas `init` bound (the engine's binding with the new engine on,
         the compositor's otherwise). */
      get canvas() {
        // Its pixels hold any drawing into the compatibility context (compat-context.ts).
        PM?.CompatContext?.flush?.();
        return (PM?.EngineCanvas?.bound ?? PM?.GL?.gl?.canvas ?? null) as HTMLCanvasElement | null;
      },
      /* The compatibility context: null until a viewer called `init`; with
         the engine on, a WebGL2 context on an offscreen canvas that reads the
         presented frame back and composites its drawing (compat-context.ts). */
      get context() { return compatContext(PM); }
    },
    raster,
    textLayout: (layer, time) => { const n = native(PM); const t = time ?? PM?.time ?? 0; return (n ? n.textLayout(layer, t) : PM?.textCaretLayout?.(layer, t)) ?? null; },
    /* The synchronous compatibility path (apiVersion 1 to 3): with the new
       engine on, pixels come from the engine through a synchronous request
       (render-transport.ts), never from the current engine. */
    renderFrameTo: (time, width, height, options) => {
      const engine = engineRender(PM);
      if (!engine) return PM?.renderFrameTo?.(time, width, height, options);
      try {
        return engine.renderFrameToCanvasSync(time, width, height, options);
      } catch (error) {
        // A Blender composition without its prepared frame fails with
        // today's words; anything else names the engine's reason.
        const e = error as { code?: string; message?: string };
        throw new Error(e?.code === 'blenderFailed' ? String(e.message) : `The engine could not render the frame: ${e?.message ?? String(error)}`);
      }
    },
    snapshot: (time, maxWidth) => {
      const engine = engineRender(PM);
      if (!engine) return PM?.Export?.snapshot?.(time, maxWidth) ?? '';
      return engine.snapshotDataUrlSync(time ?? PM?.time ?? 0, maxWidth ?? 480, PM.proj);
    },
    /* apiVersion 4's async siblings (additive): the UI never blocks. */
    /* Today's `PM.Export.snapshotAsync` with the flag on or off: with the
       engine it reaches `PM.EngineCaptures` through the exporter (and any
       wrapper of it), which prepares a Blender frame first as today. */
    snapshotAsync: async (time: number, maxWidth = 480) => {
      const engine = engineRender(PM);
      if (!engine || PM?.Export?.snapshotAsync) return (await PM?.Export?.snapshotAsync?.(time, maxWidth)) ?? '';
      return engine.snapshotDataUrl(time ?? PM?.time ?? 0, maxWidth, PM.proj, { prepareBlender: true });
    },
    renderFrameToAsync: async (time: number, width: number, height: number, options?: Record<string, unknown>) => {
      const engine = engineRender(PM);
      if (!engine) return PM?.renderFrameTo?.(time, width, height, options);
      return engine.renderFrameTo(time, width, height, options);
    },
    /* The same surface shape with the flag on or off; the crop is today's
       fifth `raster` argument. With the engine, wrappers beneath the
       service run around the engine's raster as for `raster`. */
    rasterAsync: async (layer: Layer, scale = 1, time?: number, crop?: RasterWindow | null) => {
      const engine = engineRender(PM);
      const args = [layer, scale, time, undefined, crop ?? undefined] as Parameters<RenderAPI['raster']>;
      if (!engine) return raster(...args);
      // A wrapper (a service registration, or a deprecated `PM.raster`
      // assignment through `host.pm`) runs around the engine's raster.
      const wrapped = PM?.RasterCompat?.wrapped?.() ?? (!!PM?.Kernel?.services?.get?.('raster') && PM.Kernel.services.get('raster') !== PM?.__rasterBase);
      if (wrapped) return raster(...args);
      const t = time ?? PM?.time ?? 0;
      return placed((await engineRaster(engine, rasterHost(PM), layer, Number(scale) || 1, t, crop as RasterCrop)) as ReturnType<RenderAPI['raster']>, args);
    }
  } as RenderAPI;
}

function makeUIState(PM: LegacyPM): UIStateAPI {
  return {
    timelineVersion: () => Number(PM?.UIState?.timelineVersion?.() ?? 0),
    getLayerCollapsed: (layer) => !!PM?.UIState?.getLayerCollapsed?.(layer),
    setLayerCollapsed: (layer, collapsed) => {
      const result = !!PM?.UIState?.setLayerCollapsed?.(layer, collapsed);
      // The timeline's disclosure toggle stores `layer.collapsed` (a document
      // field outside history) just before this call: the barrier records it.
      const index = Array.isArray(PM?.proj?.layers) ? PM.proj.layers.indexOf(layer) : -1;
      if (index >= 0) touchedDocument(PM, [['layers', index, 'collapsed']]);
      return result;
    },
    getGroupCollapsed: (layer) => !!PM?.UIState?.getGroupCollapsed?.(layer),
    setGroupCollapsed: (layer, collapsed) => !!PM?.UIState?.setGroupCollapsed?.(layer, collapsed),
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
    local3D: (layer, time) => attempt(() => (native(PM)?.local3D(layer, time) as any) ?? local3D(PM, layer, time), [...EMPTY_MAT4]),
    parent3D: (...args) => attempt(() => (native(PM)?.parent3D(...args) as any) ?? parent3D(PM, ...args), [...EMPTY_MAT4]),
    world3D: (...args) => attempt(() => (native(PM)?.world3D(...args) as any) ?? world3D(PM, ...args), [...EMPTY_MAT4]),
    is3DLayer: (layer) => attempt(() => { const n = native(PM); return n ? n.is3DLayer(layer) : is3DLayer(PM, layer); }, false),
    perspectiveAmount: (layer, time) => attempt(() => native(PM)?.perspectiveAmount(layer, time) ?? perspectiveAmount(PM, layer, time), 0.001),
    planeMatrix: (...args) => attempt(() => (native(PM)?.planeMatrix(...args) as any) ?? planeMatrix(PM, ...args), [...EMPTY_MAT3]),
    projectPoint: (matrix, point) => native(PM)?.projectPoint(matrix, point) ?? projectPoint(matrix, point),
    inversePlane: (matrix) => { const n = native(PM); return n ? n.inversePlane(matrix) as any : inversePlane(matrix); },
    planeContains: (layer, time, x, y, bounds) => attempt(() => { const n = native(PM); return n ? n.planeContains(layer, time, x, y, bounds) : planeContains(PM, layer, time, x, y, bounds); }, false)
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
    confirm: (title, body) => confirmPrompt(PM, { message: title, ...(body ? { detail: body } : {}) }),
    openExternal: async (value) => {
      const url = parseExtensionUrl(value);
      if (!url) throw new TypeError('ui.openExternal accepts an https URL of at most 2 KB without credentials');
      const host = hostBridge();
      // The browser host (powermove serve) opens it in the page's own browser.
      const open = host?.extensionOpenExternal ?? host?.openExternal;
      if (!open) throw new Error('ui.openExternal is unavailable in this host');
      await open.call(host, url.href);
      return true;
    },
    menu: (anchor, items: MenuContribution[] | Promise<MenuContribution[]>) => {
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
  const assets: AssetsAPI = {
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
      return blob.text();
    },
    importUrl: (async (url: string, admit?: AdmitDownload) => (await assets.import(await fetchRemoteMedia(url, undefined, admit))).id) satisfies ImportUrl
  };
  return assets;
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
      PM?.Layout?.rememberPanelOpen?.(id);
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
      const hide = PM?.Layout?.closePanel ?? PM?.Layout?.hidePanel ?? PM?.Layout?.removePanel;
      if (!hide) return;
      PM?.WS?.mutate?.((workspace: unknown) => hide(workspace, id));
    },
    isOpen: (id) => !!PM?.Layout?.hasPanel?.(current(), id),
    isHidden: (id) => {
      const hidden = ((current() as { hiddenPanels?: Array<{ id?: string }> } | undefined)?.hiddenPanels ?? []).some((item) => item.id === id);
      /* Seed the global preference from the old per-workspace behavior. This
         runs only for activation-time auto-open checks, so an intentionally
         hidden extension panel does not reappear in another project. */
      if (hidden) PM?.Layout?.rememberPanelClosed?.(id);
      return hidden || PM?.Layout?.isPanelClosed?.(id) === true;
    },
    refresh: (id) => PM?.Layout?.refresh?.(id),
    list: () => [...new Set([...kernel.panels.ids(), ...Object.keys(PM?.PANELS ?? {})])]
  };
}

function makeExtensionsAPI(PM: LegacyPM, bridge: ExtensionsHostBridge | null, getLoader: () => Loader | null): ExtensionsAPI {
  return {
    list: (): ExtensionRecord[] => storeRecords(),
    fork: async (id) => {
      if (!bridge?.fork) throw new Error('Forking built-in extensions is unavailable.');
      return bridge.fork({ id });
    },
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
    },
    rebase: (id) => {
      if (typeof PM?.requestExtensionRebase === 'function') PM.requestExtensionRebase(id);
      else PM?.cmd?.('agent');
    },
    setUp: (id) => {
      const record = storeRecords().find((item) => item.id === id);
      if (record) PM?.Vars?.openSetup?.(record);
    }
  };
}

function resolveBridge(PM: LegacyPM): ExtensionsHostBridge | null {
  const candidate = (hostBridge() as Record<string, any> | undefined)?.extensions ?? PM?.extensionsBridge ?? null;
  if (candidate && typeof candidate.list === 'function' && typeof candidate.onChanged === 'function') return candidate as ExtensionsHostBridge;
  return null;
}

function resolveVarsBridge(): Pick<VarsBridge, 'values'> | null {
  const candidate = (hostBridge() as Record<string, any> | undefined)?.vars;
  return candidate && typeof candidate.values === 'function' ? candidate as VarsBridge : null;
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
    scene3d: makeScene3DAPI(PM,(event,payload)=>kernel.events.emit(event,payload)),
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

  // Capture the native rasterizer once (it normally installs after the kernel).
  // Then leave PM.raster as an ordinary writable function so legacy wrappers
  // can capture/delegate to it without changing the base beneath themselves.
  const rasterDescriptor = Object.getOwnPropertyDescriptor(PM, 'raster');
  let nativeRaster = PM.raster;
  /* With "Use New Engine" on, a registered wrapper makes the window the
     engine's raster override producer (raster-compat.ts): the engine asks
     for each text, captions, and shape layer, and the chain runs over lazy
     engine rasters. */
  /* `PM.raster` as an extension reaches it through the deprecated
     `api.host.pm` (apiVersion 1 to 3): today the compositor calls whatever
     it holds, so a function assigned there wraps everything beneath it
     (`rasterDispatch`: the services chain over the base). It is an
     accessor so the engine's producer hears such an assignment. */
  let assignedRaster: unknown = null;
  const assignedListeners = new Set<() => void>();
  const legacyWrapper = (): RenderAPI['raster'] | null =>
    typeof assignedRaster === 'function' && assignedRaster !== rasterDispatch ? assignedRaster as RenderAPI['raster'] : null;
  const rasterCompat = createRasterCompat(PM, {
    isBase: (fn) => fn === rasterBase,
    // Today's call order: an assigned `PM.raster` outermost, then the
    // services chain (LIFO), then the base.
    chain: () => legacyWrapper() ?? kernel.services.get<RenderAPI['raster']>('raster'),
    onChainChange: (fn) => {
      const services = kernel.services.onChange('raster', fn);
      assignedListeners.add(fn);
      return { dispose: () => { services.dispose(); assignedListeners.delete(fn); } };
    },
    engineRaster: (layer, scale, time, crop) => {
      const engine = engineRender(PM);
      return engine ? engineRasterBase(PM, engine, [layer as Layer, scale, time, undefined, crop as RasterWindow | undefined]) : null;
    },
    rasterKey: (layer, scale, time, crop) => {
      // Today's raster cache (legacy/gl/raster.ts) keys pixels by resolved
      // content and density rather than composition time: rotation alone
      // never repaints a base, and a text layer whose text and style hold
      // keeps its raster. Path shapes, animated or styled characters and
      // typography anchoring depend on more than that: never cached.
      const type = layer?.type;
      if (type !== 'shape' && type !== 'text') return null;
      if (type === 'shape' && layer.d?.paths?.length) return null;
      if (type === 'text' && (layer.d?.animators?.length || layer.d?.styles?.length || layer.d?.fontAnchorBounds)) return null;
      const evaluator = native(PM);
      return evaluator ? JSON.stringify([type, layer.compat ?? null, evaluator.resolveContent(layer, time), scale, crop ?? null]) : null;
    },
    engineRasterAsync: async (layer, scale, time, crop) => {
      const engine = engineRender(PM);
      if (!engine) return null;
      return await engineRasterPrepared(engine, rasterHost(PM), layer, scale, time, crop as RasterCrop);
    }
  });
  const rasterBase: RenderAPI['raster'] = (...args) => {
    if (rasterCompat.producing) return rasterCompat.base(args);
    const engine = publicRaster > 0 ? engineRender(PM) : null;
    return engine ? engineRasterBase(PM, engine, args) : (nativeRaster?.(...args) ?? null);
  };
  PM.__rasterBase = rasterBase;
  const rasterRegistration = kernel.services.register('raster', rasterBase);
  rasterCompat.reset();
  PM.RasterCompat = rasterCompat;
  if (PM.EngineRender) subscriptions.push(rasterCompat.attach(PM.EngineRender));
  subscriptions.push(() => {
    rasterCompat.dispose();
    if (PM.RasterCompat === rasterCompat) PM.RasterCompat = null;
  });
  /* `render.gl.context` with the engine on: a WebGL2 context on an
     offscreen canvas (compat-context.ts). It hears each presented frame
     before any extension hears `frame:rendered` (this subscription is made
     before the bus is forwarded below), so drawing in a `frame:rendered`
     handler lands on the new frame, as today. */
  /* The engine connection installs after the kernel: a viewer that already
     called `render.gl.init` gets its presentation as soon as it does. */
  {
    // After the task: the connection sets `PM.EngineCanvas` right after it announces the service.
    const late = () => queueMicrotask(() => { if (bindLate(PM, initCanvas.current)) PM?.EngineCanvas?.resize?.(); });
    if (typeof window !== 'undefined') {
      window.addEventListener('powermove:engine-viewer-changed', late);
      subscriptions.push(() => window.removeEventListener('powermove:engine-viewer-changed', late));
    }
  }
  const compatContextHost = createCompatContextHost(PM);
  PM.CompatContext = compatContextHost;
  /** The viewport the last presented frame carried (today's export frames
   *  emitted `GL.previewViewport` as it stood). */
  let presentedViewport: unknown = null;
  {
    const off = PM?.bus?.on?.('preview:presented', (frame: { viewport?: unknown } | undefined) => {
      presentedViewport = frame?.viewport ?? null;
      compatContextHost.presented();
    });
    if (typeof off === 'function') subscriptions.push(off);
  }
  /* WebM exports with the engine: each frame comes here before it is
     encoded, and `frame:rendered` runs over it in the compatibility
     context, as today's `renderInto` frames did (compat-context.ts). The
     engine connection installs after the kernel and attaches its
     transport (engine-connection/install.ts). */
  const contextExports = {
    attach(transport: import('../engine-connection/render-transport').EngineRenderTransport): () => void {
      transport.contextActive = () => compatContextHost.active;
      const unprovide = transport.provideCompat('compatContext', (request) => {
        const frame = request.frame;
        if (!frame) return null;
        if (frame.payload.kind !== 'inline') throw new Error('The export frame did not reach this window.');
        const width = Number(frame.width), height = Number(frame.height);
        const time = Number(request.time) / TICKS_PER_SECOND;
        const drawn = compatContextHost.exportFrame({ width, height, data: frame.payload.bytes }, () =>
          kernel.events.emit('frame:rendered', { time, viewport: (presentedViewport ?? null) as any, version: PM.animVersion?.(), quality: 1 }));
        return drawn ? { key: `compat-context:${String(request.time)}`, width, height, format: 'rgba8Premultiplied', origin: [0, 0], scale: 1, data: drawn } : 'native';
      });
      return () => {
        unprovide();
        if (transport.contextActive) transport.contextActive = null;
      };
    }
  };
  PM.CompatContextExports = contextExports;
  if (PM.EngineRender?.provideCompat) subscriptions.push(contextExports.attach(PM.EngineRender));
  subscriptions.push(() => { if (PM.CompatContextExports === contextExports) PM.CompatContextExports = null; });
  subscriptions.push(() => {
    compatContextHost.dispose();
    if (PM.CompatContext === compatContextHost) PM.CompatContext = null;
  });
  const rasterDispatch: RenderAPI['raster'] = (...args) =>
    (kernel.services.get<RenderAPI['raster']>('raster') ?? rasterBase)(...args);
  const assignRaster = (value: unknown) => {
    if (value === assignedRaster) return;
    assignedRaster = value;
    for (const fn of [...assignedListeners]) {
      try { fn(); } catch (error) { console.error('[raster] a raster change listener failed', error); }
    }
  };
  const attachRaster = (value: any) => {
    nativeRaster = value;
    assignedRaster = rasterDispatch;
    // Reads and writes behave as the plain writable property they were
    // (`const raster = PM.raster; PM.raster = wrapper`; restoring it puts
    // `rasterDispatch` back); the setter tells the engine's producer.
    Object.defineProperty(PM, 'raster', {
      configurable: true, enumerable: true, get: () => assignedRaster, set: assignRaster
    });
  };
  if (typeof nativeRaster === 'function') attachRaster(nativeRaster);
  else Object.defineProperty(PM, 'raster', {
    configurable: true, enumerable: true, get: () => rasterDispatch, set: attachRaster
  });
  subscriptions.push(() => {
    rasterRegistration.dispose();
    if (rasterDescriptor?.get || rasterDescriptor?.set) Object.defineProperty(PM, 'raster', rasterDescriptor);
    else Object.defineProperty(PM, 'raster', {
      configurable: true, enumerable: true, writable: true, value: nativeRaster
    });
  });

  /* Route through PM.cmd when present: it is the app's public command seam
     (native menu IPC and tests interpose on it). Falls back to the kernel. */
  let inputActive = true;
  subscriptions.push(() => { inputActive = false; });
  /* App commands (Settings, the palette, windows and tabs, opening another
     project) always run. A document command waits while the document module
     loads (boot, Retry or revival) and runs once it settles; if the load
     failed it does not run and the engine's named failure shows again
     (engine-doc/readiness.ts). Trusted keys from sandboxed panels resolve
     their binding through this same runner. */
  const keyListener = kernel.installKeyListener((command, args) => runCommandWhenReady(PM, command, () => {
    if (!inputActive) return undefined;
    return typeof PM?.cmd === 'function' ? PM.cmd(command, ...(args ?? [])) : runKernelCommand(kernel, command, args);
  }));
  subscriptions.push(() => keyListener.dispose());
  const inputKeyOff = hostBridge()?.onInputKey?.(input => kernel.dispatchTrustedKey(input));
  if (inputKeyOff) subscriptions.push(inputKeyOff);

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
      // Every composition's layers may gain parameters (a metadata write).
      writeDocument(PM, [['layers'], ['comps']], () => {
        for (const container of containers) for (const layer of container?.layers ?? []) {
          if (layer?.type !== 'extension' || layer?.d?.definition !== change.id) continue;
          layer.d.params = layer.d.params && typeof layer.d.params === 'object' ? layer.d.params : {};
          for (const param of definition.params) {
            if (!layer.d.params[param.k] || typeof layer.d.params[param.k] !== 'object') {
              layer.d.params[param.k] = PM?.P?.(param.def) ?? { v: param.def, kf: [], expr: null };
            }
          }
        }
      });
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
  on('draw', () => kernel.events.emit('invalidate', 'render'));
  on('draw:timeline', () => kernel.events.emit('invalidate', 'timeline'));
  on('draw:ui', () => kernel.events.emit('invalidate', 'ui'));
  on('draw:status', () => kernel.events.emit('invalidate', 'status'));
  on('preview:presented', (frame: { viewport: PreviewViewport | null; time: number; version: number | undefined; quality: number }) =>
    kernel.events.emit('frame:rendered', { time: Number(frame?.time ?? 0), viewport: frame?.viewport ?? null, version: frame?.version, quality: Number(frame?.quality ?? 1) }));
  on('overlay', () => kernel.events.emit('overlay', undefined));

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
  kernel.services.register('compositions', {
    openTabs: () => PM.Comps?.openTabs() ?? [],
    get: (id: string) => PM.Comps?.get(id) ?? null,
    isOpen: (id: string) => PM.Comps?.isOpen(id) ?? false,
    open: (id: string) => PM.Comps?.open(id) ?? false,
    close: (id: string) => PM.Comps?.close(id) ?? false,
    onChange: (listener: () => void) => PM.bus.on('comps', listener),
  });
  // pm-edit's kernels for built-in edits that live in extensions (the
  // alignment strip): the native answer when the native evaluator is on,
  // the caller's own code otherwise (docs/engine/pm-edit.md).
  kernel.services.register('editKernels', {
    align: <T,>(layers: unknown[], mode: string, time: number, legacy: () => T): T => {
      const edits = PM.EngineDoc?.edits;
      return edits ? edits.align(layers, mode, time, legacy) : legacy();
    },
  });
  for (const location of ['viewer:context', 'timeline:context'] as const) {
    kernel.contributeMenu('core', location, () => [
      { label: 'New Composition…', icon: 'plus', kb: '⌘N', run: () => PM.cmd('newComposition') }
    ]);
  }
  return installed;
}

/**
 * Create the loader for `installKernel`'s deps and boot it. Kept separate so the
 * kernel can be installed (and used by legacy registration) before any
 * extension code runs.
 */
export async function bootExtensions(installed: InstalledKernel, builtins: Record<string, BuiltinFactory> = {}, options: Pick<LoaderOptions, 'sandboxProcess'> = {}): Promise<Loader> {
  const loader = createLoader({
    kernel: installed,
    bridge: installed.bridge,
    vars: resolveVarsBridge(),
    builtins,
    deps: installed.deps,
    ...options
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
