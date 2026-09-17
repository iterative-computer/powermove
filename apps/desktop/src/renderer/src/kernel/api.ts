/*
 * Powermove kernel API — the surface every extension programs against.
 * FROZEN for apiVersion 1. Additive changes only; breaking changes bump
 * EXTENSION_API_VERSION in src/shared/extensions.ts.
 *
 * Extensions import this as `import type { PowermoveAPI } from 'powermove'`.
 * `src/extensions/*` (built-ins) and user extensions get the same object.
 */

import type { Component } from 'svelte';
import type {
  BlendMode,
  Channel,
  ChannelValue,
  Comp,
  Effect,
  Fill,
  Keyframe,
  Layer,
  LayerType,
  Mask,
  Project
} from '../core/types/project';
import type { EditCommand, EditMeta, EditResult, JsonObject } from '../core/types/commands';
import type { WorkspaceManifest } from '../core/types/workspace';
import type { ExtensionHealth, ExtensionManifest, ExtensionRecord, ExtensionScope } from '../../../shared/extensions';

export type {
  BlendMode,
  Channel,
  ChannelValue,
  Comp,
  EditCommand,
  EditMeta,
  EditResult,
  Effect,
  Fill,
  ExtensionHealth,
  ExtensionManifest,
  ExtensionRecord,
  ExtensionScope,
  Keyframe,
  Layer,
  LayerType,
  Mask,
  Project,
  WorkspaceManifest
};

export interface Disposable {
  dispose(): void;
}

/* ── panels ──────────────────────────────────────────────── */

export interface PanelProps {
  panelId: string;
  spec: Record<string, unknown>;
  /** Present for panels registered by an extension: that extension's API. */
  api?: PowermoveAPI;
}

export interface PanelLibraryPreviewContext {
  /** Mounted live panel used only as the source of current project content. */
  source: HTMLElement;
  /** Isolated clone laid out at the definition-owned Library size. */
  clone: HTMLElement;
  width: number;
  height: number;
}

export interface PanelDefinition {
  id: string;
  title: string;
  /** Svelte component mounted into the panel body. Either `component` or `build` is required. */
  component?: Component<PanelProps>;
  /** Imperative alternative: populate `body`; return a disposer if you attach listeners. */
  build?: (body: HTMLElement, inst: { spec: Record<string, unknown> }) => void | (() => void);
  /** Optional header renderer for imperative panels. */
  header?: (header: HTMLElement, inst: Record<string, unknown>) => void;
  size?: number;
  min?: number;
  flush?: boolean;
  noscroll?: boolean;
  headless?: boolean;
  hideMoveHandle?: boolean;
  /** CSS selector inside the body that hosts the move handle. */
  moveSlot?: string;
  /** Choose a meaningful, distinct icon from the kernel icon set for every new panel. */
  icon?: string;
  /** Library presentation is definition-owned, never inferred from the active workspace layout. */
  library?: false | {
    width: number;
    height: number;
    /** Re-render stateful surfaces (for example canvas/WebGL) at the canonical size. */
    render?: (context: PanelLibraryPreviewContext) => void;
  };
}

export interface PanelsAPI {
  register(def: PanelDefinition): Disposable;
  /** Ids of all registered panels (built-in and extension). */
  list(): string[];
  /** Show the panel in a dock (adds it to the active workspace if absent). */
  open(id: string, dock?: PanelDock): void;
  open(id: string, options?: PanelOpenOptions): void;
  close(id: string): void;
  isOpen(id: string): boolean;
  /** Re-run the panel's `build` (imperative panels only). */
  refresh(id: string): void;
}

export type PanelDock = 'left' | 'center' | 'right';
export interface PanelOpenOptions { dock?: PanelDock; index?: number }

/* ── commands & keybindings ──────────────────────────────── */

export interface CommandDefinition {
  id: string;
  label: string;
  category?: string;
  /** Display hint only; bind keys with `keybindings.bind`. */
  kb?: string | null;
  run: (...args: unknown[]) => unknown;
  /** Return false to hide from palette/menus (still runnable by id). */
  when?: () => boolean;
}

export interface CommandsAPI {
  register(def: CommandDefinition): Disposable;
  run(id: string, ...args: unknown[]): unknown;
  has(id: string): boolean;
  list(): CommandDefinition[];
}

/**
 * Key chord syntax: modifiers `cmd|ctrl|alt|shift` joined by `+`, then a key
 * name: single character (`k`, `1`, `[`), or `space|enter|escape|tab|backspace|
 * delete|up|down|left|right|home|end|pageup|pagedown|f1..f12`.
 * Example: `cmd+shift+k`, `space`, `shift+f9`.
 */
export interface KeybindingDefinition {
  key: string;
  command: string;
  args?: unknown[];
  /** Fire even when a text field is focused (default false). */
  inFields?: boolean;
  /** Fire on browser key-repeat events (default false); use for continuous actions only. */
  repeat?: boolean;
  /**
   * Match when the chord's modifiers are a subset of the pressed modifiers,
   * and treat cmd/ctrl as equivalent — legacy keymap semantics.
   */
  looseModifiers?: boolean;
  /** Lower runs first; extension bindings default to 0, built-ins to 100. */
  priority?: number;
}

export interface KeybindingsAPI {
  bind(def: KeybindingDefinition): Disposable;
  /** Remove every binding for `key` (any command) registered by this extension, or all when `all` is true. */
  unbind(key: string, all?: boolean): void;
  list(): KeybindingDefinition[];
  /** Normalise a KeyboardEvent to chord syntax, or null for bare modifiers. */
  chordOf(event: KeyboardEvent): string | null;
}

/* ── effects & transitions ───────────────────────────────── */

export type EffectParamDefinition =
  | { k: string; label: string; def: number; min: number; max: number; step?: number; unit?: string; type?: 'number' }
  | { k: string; label: string; def: string; type: 'color' }
  | { k: string; label: string; def: boolean; type: 'toggle' };

/**
 * Fragment shader body contract (GLSL ES 3.00). The kernel prepends the common
 * preamble (`v_uv`, `v_st`, `u_tex`, `u_res`, `u_texel`, `u_time`, helpers) and
 * a `uniform` declaration for every param named `u_<k>` (float for numbers and
 * toggles, vec3 for colors), plus `u_pass` (int) and, when `keepOrig`, `u_orig`.
 * Output is `o` (vec4). Provide only the body of `main()` in `frag`, or a full
 * shader with `void main` when `rawShader` is true (legacy positional u_p<i>
 * uniforms remain supported for raw shaders).
 */
export interface EffectDefinition {
  id: string;
  label: string;
  group: string;
  /** At most 32 params; keys must be unique and match /^[a-z][a-zA-Z0-9]*$/. */
  params: EffectParamDefinition[];
  /** Non-empty shader body, at most 65,536 characters. */
  frag: string;
  passes?: number; // 1..8
  keepOrig?: boolean;
  rawShader?: boolean;
}

/**
 * Transition between the accumulated frame (`u_from`) and the incoming layer
 * (`u_to`) over `u_prog` ∈ [0,1]. Same preamble/param rules as effects; output `o`.
 */
export interface TransitionDefinition {
  id: string;
  label: string;
  group?: string;
  /** Same 32-param limit and key restrictions as effects. */
  params: EffectParamDefinition[];
  frag: string;
  rawShader?: boolean;
}

export interface EffectsAPI {
  register(def: EffectDefinition): Disposable;
  list(): EffectDefinition[];
  get(id: string): EffectDefinition | undefined;
}

export interface TransitionsAPI {
  register(def: TransitionDefinition): Disposable;
  list(): TransitionDefinition[];
  get(id: string): TransitionDefinition | undefined;
}

/* ── structured extension layers ────────────────────────── */

/**
 * A programmable renderer paired with a structured `.pmv` layer instance.
 * Version 1 supports fragment renderers; future renderer kinds can be added
 * without changing the persisted layer shape.
 */
export interface ExtensionLayerDefinition {
  id: string;
  label: string;
  version: number;
  icon?: string;
  color?: string;
  width?: number;
  height?: number;
  params: EffectParamDefinition[];
  defaults?: JsonObject;
  renderer: {
    kind: 'fragment';
    /** Full GLSL `void main()` body. Powermove supplies its standard layer preamble and `u_<param>` uniforms. */
    fragment: string;
  } | {
    kind: 'mesh';
    /** Key inside layer `data` containing the durable model asset id. */
    assetField: string;
  };
}

export interface ExtensionLayersAPI {
  register(definition: ExtensionLayerDefinition): Disposable;
  list(): ExtensionLayerDefinition[];
  get(id: string): ExtensionLayerDefinition | undefined;
}

/* ── durable project assets ─────────────────────────────── */

export interface AssetRecord {
  id: string;
  name: string;
  kind: string;
  type?: string;
  size?: number;
  w?: number;
  h?: number;
  dur?: number;
  format?: string;
  vertices?: number;
  triangles?: number;
  layerDefinition?: string;
  [key: string]: unknown;
}

export interface AssetsAPI {
  /** Open the system file picker. A cancellation resolves to an empty array. */
  pick(options?: { accept?: string; multiple?: boolean }): Promise<File[]>;
  /** Import into Powermove's durable project media store. The file remains after layer Undo. */
  import(file: File, options?: { layerDefinition?: string }): Promise<AssetRecord>;
  get(id: string): AssetRecord | undefined;
  /** Read a text asset from the live cache or durable media store. */
  readText(id: string): Promise<string>;
}

/* ── themes ──────────────────────────────────────────────── */

export interface ThemeDefinition {
  id: string;
  name: string;
  /** `light` | `dark` | `auto` (follows system, tokens may provide both). */
  scheme: 'light' | 'dark' | 'auto';
  /** CSS custom properties applied on :root, e.g. { '--accent': '#f60' }. Keys must start with `--`. */
  tokens?: Record<string, string>;
  /** Dark-scheme overrides when scheme is `auto`. */
  darkTokens?: Record<string, string>;
  /** Arbitrary stylesheet text injected while the theme is active (last wins). */
  css?: string;
  /** Extra attributes to set on <html>, e.g. { 'data-win98': '' }. */
  rootAttributes?: Record<string, string>;
}

export interface ThemeAPI {
  register(def: ThemeDefinition): Disposable;
  activate(id: string): void;
  active(): string;
  list(): ThemeDefinition[];
  /** Kernel scheme preference (System/Light/Dark) as chosen in Settings. */
  scheme(): 'light' | 'dark' | 'system';
  setScheme(mode: 'light' | 'dark' | 'system'): void;
}

/* ── palette, menus, status ──────────────────────────────── */

export interface PaletteEntry {
  id: string;
  label: string;
  category: string;
  kb?: string | null;
  run(): unknown;
}
export type PaletteProvider = (query: string) => PaletteEntry[];

export type MenuLocation = 'titlebar:right' | 'panel:context' | 'layer:context' | 'timeline:context' | 'viewer:context';

export type MenuContribution =
  | '-'
  | { header: string }
  | { label: string; icon?: string; kb?: string | null; on?: boolean; disabled?: boolean; run?: () => unknown };

export interface MenusAPI {
  contribute(location: MenuLocation, items: (ctx: Record<string, unknown>) => MenuContribution[]): Disposable;
  /** Everything contributed for a location, in registration order. */
  collect(location: MenuLocation, ctx?: Record<string, unknown>): MenuContribution[];
}

export interface StatusItem {
  id: string;
  /** Return text (or null to hide). Called on every status tick. */
  text: () => string | null;
  title?: string;
  side?: 'left' | 'right';
  onClick?: () => void;
}

export interface StatusAPI {
  register(item: StatusItem): Disposable;
  list(): StatusItem[];
}

export interface PaletteAPI {
  registerProvider(provider: PaletteProvider): Disposable;
  open(query?: string): void;
}

/* ── project façade ──────────────────────────────────────── */

export interface Selection {
  layers: string[];
  keys: string[];
  chan: string | null;
}

export interface ProjectAPI {
  /** Current project object graph. Treat as read-only; mutate through `apply`. */
  get(): Project;
  revision(): number;
  /** Typed, validated, undoable edit. `meta.origin` is forced to `ext:<id>`. */
  apply(commands: EditCommand | EditCommand[], meta?: Omit<EditMeta, 'origin'>): EditResult;
  selection(): Selection;
  select(layerIds: string[], add?: boolean): void;
  time(): number;
  setTime(t: number): void;
  play(): void;
  pause(): void;
  playing(): boolean;
  undo(): void;
  redo(): void;
  /** Render the current composition at `t` to a JPEG data URL (≤ maxWidth px). */
  snapshot(t?: number, maxWidth?: number): Promise<string>;
}

/* ── editor kernel façades ──────────────────────────────── */

export type AffineMatrix = [number, number, number, number, number, number];
export type Mat3 = [number, number, number, number, number, number, number, number, number];
export type Mat4 = [number, number, number, number, number, number, number, number, number, number, number, number, number, number, number, number];
export interface Point { x: number; y: number }
export interface Bounds { x0: number; x1: number; y0: number; y1: number; cx?: number; cy?: number }
export interface PropertyEntry { key: string; prop: Channel; label: string; group: string }

/**
 * Animation evaluates channels and hierarchy transforms without changing the project, except for the key/easing helpers and `touch`, which mutate channel data and invalidate animation-derived caches. Expression evaluation records failures in `expressionErrors` but does not emit project events.
 */
export interface AnimAPI {
  ev(layer: Layer, key: string, time: number): ChannelValue | null;
  evP(layer: Layer, prop: Channel, time: number, key: string): ChannelValue | null;
  active(layer: Layer, time: number): boolean;
  findProp(layer: Layer, key: string): Channel | null;
  allProps(layer: Layer): PropertyEntry[];
  hasKeyAt(layer: Layer, prop: Channel, time: number): Keyframe | null;
  setKey(layer: Layer, key: string, time: number, value: ChannelValue, ease?: string): Keyframe | null;
  setKeyOn(prop: Channel, localTime: number, value: ChannelValue, ease?: string, fps?: number): Keyframe | null;
  removeKey(prop: Channel, key: Keyframe): void;
  applyEaseTo(keys: Keyframe[], name: string): void;
  wouldCycle(layer: Layer, parentId: string | null): boolean;
  resolveContent(layer: Layer, time?: number): Layer['d'];
  readonly expressionErrors: WeakMap<object, string>;
  version(): number;
  touch(): void;
  worldMatrix(layer: Layer, time: number): AffineMatrix;
  localMatrix(layer: Layer, time: number): AffineMatrix;
  transformParentMatrix(layer: Layer, time: number, parent?: Layer | null): AffineMatrix;
  mul(left: Readonly<AffineMatrix>, right: Readonly<AffineMatrix>): AffineMatrix;
}

export interface ChannelDefinition { label: string; group: string; unit?: string; step?: number; min?: number; max?: number }
export interface LayerFactoryOptions {
  name?: string;
  from?: number;
  dur?: number;
  color?: string;
  p?: Partial<Record<string, ChannelValue>>;
  d?: Partial<Layer['d']>;
}

/**
 * Model exposes the canonical factories, schema tables, and current-composition lookups. Factories allocate detached project objects; lookups are read-only, and callers must use edit/history APIs to make durable project mutations.
 */
export interface ModelAPI {
  P<T extends ChannelValue>(value: T, options?: Partial<Channel<T>>): Channel<T>;
  readonly CH: Readonly<Record<string, ChannelDefinition>>;
  KF<T extends ChannelValue>(time: number, value: T, ease?: string): Keyframe<T>;
  readonly BLENDS: readonly BlendMode[];
  readonly TYPE_META: typeof import('../core/types/project').TYPE_META;
  readonly MASK_SHAPES: readonly Mask['shape'][];
  mkLayer(type: LayerType, options?: LayerFactoryOptions, comp?: Comp): Layer;
  mkMask(shape?: Mask['shape'], comp?: Comp): Mask;
  mkProject(options?: Partial<Project>): Project;
  cloneLayer(layer: Layer): Layer;
  normalizeFill(value: unknown, fallback?: string): Fill;
  layerDefinition(id: string): ExtensionLayerDefinition | undefined;
  curComp(): Comp;
  layer(id: string): Layer | null;
  byName(name: string): Layer | null;
}

/**
 * Selection reads and updates the single mutable legacy selection object so existing references stay live. `set` and `select` emit the legacy `sel` event and invalidate dependent UI; selection history remains suppressed while a project transaction is pending by the backing history service.
 */
export interface SelectionAPI {
  get(): Selection;
  layers(): string[];
  first(): Layer | null;
  keys(): string[];
  chan(): string | null;
  set(partial: Partial<Selection>): void;
  select(ids: string[], add?: boolean): void;
  resolveSelectedKeys(): Keyframe[];
  keySelectionActive: boolean;
}

/**
 * Groups query hierarchy and timing, expand selections, and perform structural reparenting. `normalizeStack` and `moveToGroup` mutate the project; the latter also touches animation state and emits the legacy layer/selection changes.
 */
export interface GroupsAPI {
  ancestors(layer: Layer, layers?: Layer[]): Layer[];
  transformRoots(ids: string[]): Layer[];
  /** Bounds of a group's active visual descendants in the group's local space. */
  bounds(group: Layer, time: number): (Bounds & { w: number; h: number; ax: number; ay: number }) | null;
  span(group: Layer): { from: number; dur: number };
  expand(ids: string[]): string[];
  normalizeStack(): void;
  moveToGroup(ids: string[], target: string | null): { ids: string[]; group: string | null };
}

export interface TransportPerformance { fps: number; ms: number; drops: number; budget: number; auto: boolean }
export interface SetTimeOptions { raw?: boolean; force?: boolean }

/**
 * Transport owns mutable playhead, playback and preview-quality state. Changing time or playback emits the backing transport events and invalidates rendering/UI; `invalidate` preserves the legacy immediate-render and coalesced UI invalidation behavior.
 */
export interface TransportAPI {
  time(): number;
  setTime(time: number, options?: SetTimeOptions): void;
  play(): void;
  pause(): void;
  toggle(): void;
  playing(): boolean;
  step(frames: number): void;
  quality: number;
  readonly perf: TransportPerformance;
  invalidate(what?: string): void;
  previewResolution: string | number | null;
}

export interface HistoryExternalOptions { bytes?: number; cleanup?: () => void; group?: string | null }

/**
 * History brackets raw mutations, commits undo/redo patches, and publishes the same project invalidations as the legacy stack. Selection entries are ignored while another transaction is pending, preserving atomic edit behavior.
 */
export interface HistoryAPI {
  do<T>(label: string, fn: () => T): T;
  begin(label: string, group?: string | null): unknown;
  commit(label?: string): boolean;
  cancel(): void;
  undo(): boolean;
  redo(): boolean;
  external(label: string, undo: () => void, redo: () => void, options?: HistoryExternalOptions): string | null;
  selection(before: Selection, after: Selection): string | null;
}

/**
 * Edit is the validated, provenance-aware project mutation surface. One-shot `apply` calls and live begin/dispatch/commit gestures create undoable edits, while `cancel` rolls changed gestures back and `mutate` wraps structural project changes in one atomic history transaction.
 */
export interface EditAPI {
  apply(input: EditCommand | EditCommand[], meta?: EditMeta): EditResult;
  begin(label: string, meta?: EditMeta): void;
  commit(label?: string): EditResult;
  cancel(): boolean;
  dispatch(command: EditCommand): EditResult;
  mutate<T>(label: string, action: () => T, meta?: EditMeta): T | EditResult;
}

export interface ImportPlacement { at: number; index?: number }
export interface ImportFilesOptions { project?: Project; placement?: ImportPlacement | null; sequence?: boolean; replaceAssetId?: string }
export interface RuntimeAsset extends AssetRecord { blob?: Blob; sourceText?: string; [key: string]: unknown }
export interface WaveformOptions { [key: string]: unknown }
export interface FontCatalog {
  bundled: string[];
  system: string[];
  families: string[];
  setSystemFamilies(values: string[]): void;
  options(current: string): string[];
  ensure(family: string, weight?: number): Promise<void>;
}

/**
 * Media exposes timing queries, file import, layer-command creation, waveform drawing, runtime raster assets and fonts. Imports/assets/fonts may perform asynchronous I/O and invalidate caches; command creation and timing helpers do not mutate the project by themselves.
 */
export interface MediaAPI {
  readonly timing: { isTimed(layer: Layer): boolean; rate(layer: Layer): number; earliestStart(layer: Layer): number };
  importFiles(files: FileList | File[], options?: ImportFilesOptions): Promise<unknown>;
  commandForAsset(id?: string, at?: number): EditCommand | undefined;
  readonly audio: { drawWaveform(ctx: CanvasRenderingContext2D, layer: Layer, options?: WaveformOptions): boolean };
  readonly assets: {
    get(id: string): RuntimeAsset | undefined;
    add(file: File, options?: Record<string, unknown>): Promise<RuntimeAsset>;
    kind(file: File): 'image' | 'video' | 'audio' | 'model' | null;
  };
  readonly fonts: FontCatalog;
}

export interface PreviewViewport { x: number; y: number; width: number; height: number; [key: string]: unknown }
export interface RasterWindow { x?: number; y?: number; w?: number; h?: number; [key: string]: unknown }
export interface RasterSurface { cv?: HTMLCanvasElement; canvas?: HTMLCanvasElement; bitmap?: ImageBitmap; anchorX?: number; anchorY?: number; x?: number; y?: number; w?: number; h?: number; [key: string]: unknown }

/**
 * Render provides WebGL setup, picking/bounds, raster cache access and frame capture. Rendering may allocate or invalidate GPU/CPU caches and temporarily change preview quality, but it does not make durable project edits.
 */
export interface RenderAPI {
  readonly gl: {
    bounds(layer: Layer, time: number): Bounds | null;
    pick(x: number, y: number, time: number, options?: { includeLocked?: boolean }): Layer | null;
    init(canvas: HTMLCanvasElement, options?: { alpha?: boolean; quiet?: boolean }): boolean;
    resize(width: number, height: number, previewViewport?: PreviewViewport | null): boolean;
    compileError(key: string): string | null;
    readonly previewViewport: PreviewViewport | null;
    readonly context: WebGL2RenderingContext | null;
  };
  raster(layer: Layer, scale?: number, time?: number, uploaded?: (key: string) => unknown, crop?: RasterWindow): RasterSurface | null;
  renderFrameTo(time: number, width: number, height: number, options?: Record<string, unknown>): HTMLCanvasElement;
  snapshot(time: number, maxWidth?: number): string;
}

export interface KeyHandleState { [key: string]: unknown }
export interface ShaderMeta { [key: string]: unknown }

/**
 * UI state stores non-project disclosure, graph-handle, reveal and shader metadata. Setters mutate only the backing UI-state caches and compatibility fields; they do not create project history entries or render output directly.
 */
export interface UIStateAPI {
  getLayerCollapsed(layer: Layer): boolean;
  setLayerCollapsed(layer: Layer, collapsed: boolean): boolean;
  getGroupCollapsed(layer: Layer): boolean;
  setGroupCollapsed(layer: Layer, collapsed: boolean): boolean;
  getKeyHandles(key: Keyframe): KeyHandleState | null;
  setKeyHandles(key: Keyframe, patch: KeyHandleState): KeyHandleState | null;
  getFxOpen(effect: Effect): boolean;
  setFxOpen(effect: Effect, open: boolean): boolean;
  getReveal(layer: Layer): string[] | null;
  setReveal(layer: Layer, keys: string[]): string[] | null;
  getShaderMeta(layer: Layer): ShaderMeta | null;
  setShaderMeta(layer: Layer, patch: ShaderMeta): ShaderMeta | null;
}

export interface DragOptions {
  move(dx: number, dy: number, event: PointerEvent): void;
  up?(event?: PointerEvent): void;
  cancel?(): void;
  cursor?: string;
  infinite?: boolean;
}
export interface EditGestureAPI { begin(): void; write(value: unknown): void; commit(): void; cancel(): void; once(value: unknown): unknown }
export interface EditGestureConstructor { new(binding: ControlEditBinding): EditGestureAPI }

export interface AssetDragPayload { id: string; name: string; kind: string; dur?: number }
export interface FxDragPayload { kind: 'effect' | 'transition'; id: string; label: string }

/**
 * Drag-and-drop serializes asset/FX payloads to the canonical MIME keys and proxies the live in-process media drag. Reads are side-effect free; `startAssetDrag` writes the DataTransfer, while `applyFxDrop` performs the backing undoable edit and invalidates the viewer.
 */
export interface DndAPI {
  readonly ASSET_MIME: string;
  readonly FX_MIME: string;
  startAssetDrag(dataTransfer: DataTransfer | null | undefined, payload: AssetDragPayload): void;
  mediaDrag: AssetDragPayload | null;
  hasAssetDrag(dataTransfer: DataTransfer | null | undefined): boolean;
  hasFileDrag(dataTransfer: DataTransfer | null | undefined): boolean;
  hasMediaDrag(dataTransfer: DataTransfer | null | undefined): boolean;
  readAssetDrag(dataTransfer: DataTransfer | null | undefined): AssetDragPayload | null;
  hasFxDrag(dataTransfer: DataTransfer | null | undefined): boolean;
  readFxDrag(dataTransfer: DataTransfer | null | undefined): FxDragPayload | null;
  applyFxDrop(payload: FxDragPayload, layerId: string | null | undefined, edge?: 'in' | 'out'): boolean;
}

export interface WorkspaceMutateOptions { inPlace?: boolean }

/**
 * Workspace exposes the active layout and persists mutations through the legacy workspace manager, including built-in-to-user cloning unless `inPlace` is requested. Panel operations mutate layout state, normalize dock fill, apply the layout and emit workspace/layout events; they do not touch project history.
 */
export interface WorkspaceAPI {
  current(): WorkspaceManifest | null;
  mutate(fn: (workspace: WorkspaceManifest) => void, options?: WorkspaceMutateOptions): WorkspaceManifest | null;
  hasPanel(id: string): boolean;
  addPanel(id: string, dock?: PanelDock, index?: number): void;
  movePanel(id: string, dock: PanelDock, index: number): boolean;
  removePanel(id: string): void;
  hidePanel(id: string): boolean;
  restorePanel(id: string): boolean;
  refresh(id: string): void;
}

/**
 * Utility contains deterministic numeric, timecode, colour and identifier helpers. Calls are otherwise side-effect free; only `uid` consumes runtime randomness to allocate a new identifier.
 */
export interface UtilAPI {
  round(value: number, places?: number): number;
  clamp(value: number, min: number, max: number): number;
  lerp(from: number, to: number, amount: number): number;
  snapF(time: number, fps: number): number;
  tc(seconds: number, fps?: number, showFrames?: boolean): string;
  parseTc(value: string, fps?: number): number | null;
  uid(prefix?: string): string;
  hex2rgb(hex: string): number[];
  rgb2hex(red: number, green: number, blue: number): string;
}

/**
 * Ease exposes the immutable preset table and identifies matching temporal handles. Both operations are read-only and do not mutate channels, history, or render state.
 */
export interface EaseAPI { nameOf(easeOut: number[], easeIn: number[]): string; readonly PRESETS: Readonly<Record<string, number[]>> }

/**
 * Space3D computes bound-PM layer, parent and perspective transforms plus plane projection/containment. These helpers are read-only; the adapter supplies the legacy PM argument so extensions never receive or pass the registry.
 */
export interface Space3DAPI {
  readonly CHANNELS_3D: Readonly<Record<string, number>>;
  local3D(layer: Layer, time: number): Mat4;
  parent3D(layer: Layer, time: number, parent?: Layer | null): Mat4;
  world3D(layer: Layer, time: number, positioning?: boolean): Mat4;
  is3DLayer(layer: Layer): boolean;
  perspectiveAmount(layer: Layer, time: number): number;
  planeMatrix(layer: Layer, time: number, positioning?: boolean): Mat3;
  projectPoint(matrix: Readonly<Mat3>, point: Point): Point;
  inversePlane(matrix: Readonly<Mat3>): Mat3 | null;
  planeContains(layer: Layer, time: number, x: number, y: number, bounds: Bounds): boolean;
}

/* ── ui helpers ──────────────────────────────────────────── */

export type ControlComponent = Component<Record<string, unknown>>;

export type ControlEditBinding =
  | { mode: 'command'; label: string; origin?: string; prepare?: () => void; command: EditCommand | ((value: unknown) => EditCommand | EditCommand[]) }
  | { mode: 'local'; label: string; set(value: unknown): void }
  | { mode: 'set'; label: string; set(value: unknown): void };

export interface ControlBindingOptions {
  label?: string;
  origin?: string;
}

export interface ControlsAPI {
  /** Kernel-provided control components; props match src/renderer/src/controls — stable within apiVersion 1. */
  readonly NumField: ControlComponent;
  readonly ColorField: ControlComponent;
  readonly FillField: ControlComponent;
  readonly FontField: ControlComponent;
  readonly SelectField: ControlComponent;
  readonly TextField: ControlComponent;
  readonly ToggleField: ControlComponent;
  readonly Row: ControlComponent;
  readonly Section: ControlComponent;
  /**
   * UNSTABLE. Binding factories used by built-in property editors. Their
   * returned binding shape is versioned, but helper arguments may grow.
   */
  readonly binding: {
    channelBinding(
      layerId: string,
      channel: string,
      options?: ControlBindingOptions & { time?: number | (() => number) }
    ): ControlEditBinding;
    layerFieldBinding(layerId: string, field: string, options?: ControlBindingOptions): ControlEditBinding;
    contentBinding(layerId: string, field: string, options?: ControlBindingOptions): ControlEditBinding;
    compositionBinding(field: string, options?: ControlBindingOptions): ControlEditBinding;
  };
}

/**
 * UI exposes kernel controls, overlays, menus and pointer helpers. Most members only mutate transient interface state; parent picking can apply an edit, shader opening mutates workspace state, and gesture coordinates the backing edit/history transaction.
 */
export interface UIAPI {
  readonly controls: ControlsAPI;
  toast(
    text: string,
    opts?: {
      /** State the outcome rather than letting the message text be sniffed, so a
          success that quotes a user-chosen name stays a success. */
      error?: boolean;
      sticky?: boolean;
      dismissible?: boolean;
      icon?: string;
      /** Replace an earlier toast with the same key instead of stacking. */
      key?: string;
      /** Persistent notices sit in a corner; status toasts stay bottom-center. */
      corner?: 'top-right' | 'bottom-right';
      /** One primary action rendered inside the toast. */
      action?: { label: string; run: () => void };
      /** Called only on explicit user dismissal. */
      onDismiss?: () => void;
    }
  ): void;
  confirm(title: string, body?: string): Promise<boolean>;
  menu(anchor: HTMLElement | { x: number; y: number }, items: MenuContribution[]): void;
  modal(opts: { title?: string; body?: HTMLElement | string; width?: number; actions?: Array<{ label: string; pri?: boolean; run?: () => unknown }> }): { close(): void; body: HTMLElement };
  /** Icon SVG markup by name from the kernel set. */
  icon(name: string): string;
  drag(event: PointerEvent, options: DragOptions): { cancel(): void };
  closeMenus(): void;
  showLayerMenu(layer: Layer, event: { clientX: number; clientY: number }, origin?: string): void;
  showParentMenu(ids: string[], event: { clientX: number; clientY: number }): HTMLElement | undefined;
  beginParentPick(event: PointerEvent, ids: string[]): void;
  openShaderEditor(layer?: Layer): void;
  readonly gesture: EditGestureConstructor;
}

export interface SourcePreview {
  show(assetId: string): boolean;
  toggle(assetId: string): void;
  clear(): void;
  dispose(): void;
  readonly activeId: string | null;
  readonly playing: boolean;
}
export interface SnapAxisCandidate { value: number; point: Point; [key: string]: unknown }
export interface SnapCandidates { x: SnapAxisCandidate[]; y: SnapAxisCandidate[] }

export interface TimelineService {
  graph: boolean;
  cv: HTMLCanvasElement | null;
  pps: number;
  scrollY: number;
  scrollT: number;
  keySelectionActive: boolean;
  nextEdge(): number;
  prevEdge(): number;
  layerAtPoint(clientX: number, clientY: number): Layer | null;
  frameView(): void;
  reveal(layer: Layer, keys: string[]): void;
}
export interface ViewerService {
  pan: number[];
  zoom: number;
  fit: boolean;
  shown: number;
  showControls?: boolean;
  preview?: SourcePreview;
  layout(panOnly?: boolean): void;
  stage: HTMLElement | null;
  ov: HTMLCanvasElement | null;
  attach(stage: HTMLElement): ViewerService | void;
  worldBounds(layer: Layer, time: number): Bounds | null;
  snapshotSnapCandidates(time: number, selectionLayers: Layer[]): SnapCandidates;
  isNavigating(): boolean;
  deferNavigationRender(now: number): boolean;
  setZoom(zoom: number): number | false;
  layerAtPoint?(clientX: number, clientY: number): Layer | null;
  /** Layer id whose source text is being edited on the canvas; the compositor skips drawing it. */
  canvasTextEditing?: string | null;
}
export interface InspectorService {
  refresh(): void;
  syncs: unknown[];
  focusText(layer: Layer): void;
  copySelectedEffects(): boolean;
  clearEffectClipboard(): void;
  pasteCopiedEffects(): boolean;
  body: HTMLElement | null;
}
export interface ToolService { tool: string; toolShape: string; setTool(tool: string, detail?: string): void }
export interface ShaderHooks { syncShaderUniforms(layer: Layer): void }

/**
 * Services is a typed LIFO compatibility registry for extension-owned runtime capabilities. Registering the same name shadows the prior implementation; disposing restores it. Registration itself has no project side effects.
 * The host registers `raster` (RenderAPI['raster']). Capture that service before
 * wrapping it; calling api.render.raster from inside its override would recurse.
 * Native surfaces expose their pixels as `cv`; preserve key, dimensions, anchor
 * and selection geometry. Overrides affect preview and export and must delegate
 * unaffected layers to the captured service.
 */
export interface ServicesAPI {
  register<T>(name: string, implementation: T): Disposable;
  get<T>(name: string): T | null;
}

/* ── storage ─────────────────────────────────────────────── */

export interface StorageAPI {
  get<T = unknown>(key: string): T | undefined;
  set(key: string, value: unknown): void;
  delete(key: string): void;
}

/* ── events ──────────────────────────────────────────────── */

export interface KernelEvents {
  'project:changed': { kind: 'values' | 'structure' | 'project' | 'assets' | 'library' | 'history' | 'replace' };
  selection: Selection;
  time: number;
  transport: { playing: boolean };
  /** Font family catalogue changed; payload is the complete ordered families list. */
  fonts: string[];
  layout: undefined;
  'theme:changed': { id: string; scheme: 'light' | 'dark' };
  'extension:loaded': { id: string };
  'extension:unloaded': { id: string };
  /** Any extension record changed (health, enablement, rebuild) — re-read `extensions.list()`. */
  'extensions:changed': { ids: string[]; reason: string };
  /** The compositor presented a frame. `viewport` is the exact preview viewport
   * object it drew into (identity-comparable with `render.gl.previewViewport`),
   * so a viewer can tell a fresh presentation from a reused one. */
  'frame:rendered': { time: number; viewport: PreviewViewport | null; version: number | undefined; quality: number };
  /** The engine finished a frame and overlays (selection, guides) should redraw. */
  overlay: undefined;
  /** Coalesced repaint request from the host: 'render' fires immediately for
   * the compositor; 'timeline', 'ui', and 'status' fire once per animation
   * frame after any host-side invalidation (reveal, collapse, history restore,
   * selection restore). Extensions that cache derived rows or panels rebuild on it. */
  invalidate: 'render' | 'timeline' | 'ui' | 'status';
}

export interface EventsAPI {
  on<K extends keyof KernelEvents>(event: K, fn: (payload: KernelEvents[K]) => void): Disposable;
  emit<K extends keyof KernelEvents>(event: K, payload: KernelEvents[K]): void;
}

/* ── extensions (introspection) ──────────────────────────── */

export interface ExtensionsAPI {
  list(): ExtensionRecord[];
  fork(id: string): Promise<{ id: string }>;
  setEnabled(id: string, enabled: boolean): Promise<void>;
  remove(id: string): Promise<void>;
  reload(id: string): Promise<void>;
  reveal(id: string): Promise<void>;
  /** Ask the agent to repair a failing extension (opens the agent panel with a prefilled prompt). */
  requestFix(id: string): void;
  /** Ask the agent to rebase a stale user fork onto its newly shipped built-in. */
  rebase(id: string): void;
}

/* ── the API object ──────────────────────────────────────── */

export interface HostAPI {
  /**
   * @deprecated Use the typed top-level API namespaces instead.
   *
   * UNSTABLE. The legacy `PM` registry. Built-ins use it mid-migration; user
   * extensions should prefer the typed surface. Shape may change between app
   * versions without an apiVersion bump.
   */
  readonly pm: unknown;
  /**
   * @deprecated Use the typed top-level API namespaces instead.
   *
   * UNSTABLE. Escape hatch to the renderer's rune stores. Built-ins use it
   * mid-migration; shapes may change between app versions without an
   * apiVersion bump.
   */
  readonly state: {
    readonly doc: unknown;
    readonly sel: unknown;
    readonly transport: unknown;
    readonly perf: unknown;
  };
  /** Mount a Svelte component into an element (kernel-provided svelte runtime). */
  mount<P extends Record<string, unknown>>(component: Component<P>, target: HTMLElement, props: P): () => void;
}

export interface PowermoveAPI {
  readonly id: string; // extension id
  readonly apiVersion: 1;
  readonly manifest: ExtensionManifest;

  readonly panels: PanelsAPI;
  readonly commands: CommandsAPI;
  readonly keybindings: KeybindingsAPI;
  readonly effects: EffectsAPI;
  readonly transitions: TransitionsAPI;
  readonly layers: ExtensionLayersAPI;
  readonly assets: AssetsAPI;
  readonly theme: ThemeAPI;
  readonly palette: PaletteAPI;
  readonly menus: MenusAPI;
  readonly status: StatusAPI;
  readonly project: ProjectAPI;
  readonly anim: AnimAPI;
  readonly model: ModelAPI;
  readonly selection: SelectionAPI;
  readonly groups: GroupsAPI;
  readonly transport: TransportAPI;
  readonly history: HistoryAPI;
  readonly edit: EditAPI;
  readonly media: MediaAPI;
  readonly render: RenderAPI;
  readonly uiState: UIStateAPI;
  readonly ui: UIAPI;
  readonly dnd: DndAPI;
  readonly workspace: WorkspaceAPI;
  readonly util: UtilAPI;
  readonly ease: EaseAPI;
  readonly space3d: Space3DAPI;
  readonly services: ServicesAPI;
  readonly storage: StorageAPI;
  readonly events: EventsAPI;
  readonly extensions: ExtensionsAPI;
  readonly host: HostAPI;

  /** Shorthand for events.on. */
  on: EventsAPI['on'];
  log(level: 'info' | 'warn' | 'error', message: string, ...data: unknown[]): void;
  /** Register a disposer to run on deactivate. */
  onDispose(fn: () => void): void;
}

export type ActivateFn = (api: PowermoveAPI) => void | Disposable | Promise<void | Disposable>;

export interface ExtensionModule {
  default: ActivateFn;
  /** Optional explicit deactivate; disposables registered through `api` are always released. */
  deactivate?: () => void | Promise<void>;
}

export * from './editor-helpers';
