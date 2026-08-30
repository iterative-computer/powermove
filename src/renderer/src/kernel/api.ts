/*
 * Powermove kernel API — the surface every extension programs against.
 * FROZEN for apiVersion 1. Additive changes only; breaking changes bump
 * EXTENSION_API_VERSION in src/shared/extensions.ts.
 *
 * Extensions import this as `import type { PowermoveAPI } from 'powermove'`.
 * `src/extensions/*` (built-ins) and user extensions get the same object.
 */

import type { Component } from 'svelte';
import type { Project } from '../core/types/project';
import type { EditCommand, EditMeta, EditResult } from '../core/types/commands';
import type { ExtensionHealth, ExtensionManifest, ExtensionRecord, ExtensionScope } from '../../../shared/extensions';

export type { Project, EditCommand, EditMeta, EditResult, ExtensionHealth, ExtensionManifest, ExtensionRecord, ExtensionScope };

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
  open(id: string, dock?: 'left' | 'center' | 'right'): void;
  close(id: string): void;
  isOpen(id: string): boolean;
  /** Re-run the panel's `build` (imperative panels only). */
  refresh(id: string): void;
}

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
  params: EffectParamDefinition[];
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
  | { label: string; kb?: string | null; on?: boolean; disabled?: boolean; run?: () => unknown };

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
      pm: Record<string, any>,
      layerId: string,
      channel: string,
      options?: ControlBindingOptions & { time?: number | (() => number) }
    ): ControlEditBinding;
    layerFieldBinding(pm: Record<string, any>, layerId: string, field: string, options?: ControlBindingOptions): ControlEditBinding;
    contentBinding(pm: Record<string, any>, layerId: string, field: string, options?: ControlBindingOptions): ControlEditBinding;
    compositionBinding(pm: Record<string, any>, field: string, options?: ControlBindingOptions): ControlEditBinding;
  };
}

export interface UIAPI {
  readonly controls: ControlsAPI;
  toast(text: string, opts?: { sticky?: boolean }): void;
  confirm(title: string, body?: string): Promise<boolean>;
  menu(anchor: HTMLElement | { x: number; y: number }, items: MenuContribution[]): void;
  modal(opts: { title?: string; body?: HTMLElement | string; width?: number; actions?: Array<{ label: string; pri?: boolean; run?: () => unknown }> }): { close(): void; body: HTMLElement };
  /** Icon SVG markup by name from the kernel set. */
  icon(name: string): string;
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
  layout: undefined;
  'theme:changed': { id: string; scheme: 'light' | 'dark' };
  'extension:loaded': { id: string };
  'extension:unloaded': { id: string };
  /** Any extension record changed (health, enablement, rebuild) — re-read `extensions.list()`. */
  'extensions:changed': { ids: string[]; reason: string };
  'frame:rendered': { time: number };
}

export interface EventsAPI {
  on<K extends keyof KernelEvents>(event: K, fn: (payload: KernelEvents[K]) => void): Disposable;
  emit<K extends keyof KernelEvents>(event: K, payload: KernelEvents[K]): void;
}

/* ── extensions (introspection) ──────────────────────────── */

export interface ExtensionsAPI {
  list(): ExtensionRecord[];
  setEnabled(id: string, enabled: boolean): Promise<void>;
  remove(id: string): Promise<void>;
  reload(id: string): Promise<void>;
  reveal(id: string): Promise<void>;
  /** Ask the agent to repair a failing extension (opens the agent panel with a prefilled prompt). */
  requestFix(id: string): void;
}

/* ── the API object ──────────────────────────────────────── */

export interface HostAPI {
  /**
   * UNSTABLE. The legacy `PM` registry. Built-ins use it mid-migration; user
   * extensions should prefer the typed surface. Shape may change between app
   * versions without an apiVersion bump.
   */
  readonly pm: unknown;
  /**
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
  readonly theme: ThemeAPI;
  readonly palette: PaletteAPI;
  readonly menus: MenusAPI;
  readonly status: StatusAPI;
  readonly project: ProjectAPI;
  readonly ui: UIAPI;
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
