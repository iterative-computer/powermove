/**
 * The contract between a panel's `mount` / extension's `activate` and the
 * window that hosts it. Built-in panels mount through `BuiltinEnv`, and user
 * extensions receive the same environment plus all 6 capability layers
 * (`powermove/builtin`).
 */
import type { HostClient } from "./host-api";
import type { Json, Transport } from "./protocol";
import type { SurfaceRoot } from "./renderer";

export type Capability =
  | "commands"
  | "themes"
  | "menus"
  | "overlays"
  | "timeline"
  | "effects"
  | "transitions"
  | "layers"
  | "toolbar"
  | "toolbar.modify"
  | "toolbar.contributeAddItem"
  | "panelHeader"
  | "panelHeader.modify"
  | "menuBar"
  | "statusBar"
  | "statusBar.contribute"
  | "statusBar.modify"
  | "ui"
  | "ui.surfaces"
  | "tools"
  | "slots"
  | "events"
  | "workspace"
  | "workspace.setLayout"
  | "dialogs"
  | "fs"
  | "storage"
  | "net"
  | "clipboard"
  | "media"
  | "shell";

export interface ApiInfo {
  readonly version: number;
  readonly capabilities: readonly Capability[];
  has(capability: string): boolean;
}

export interface BuiltinEnv {
  /** The panel id this mount draws (the built-in's id for a fork). */
  panel: string;
  /** Synchronous host operations of this panel's realm. */
  native(operation: string, input?: unknown): any;
  /** An AppKit / native host request answered asynchronously. */
  platform(operation: string, input?: unknown): Promise<any>;
  host: HostClient;
  transport: Transport;
  rootOptions: { generation: number };
  /** A new surface root on this panel's transport. */
  root(): SurfaceRoot;
  /** A kernel command through the host client; resolves with its value. */
  command<T = Json | undefined>(name: string, args?: unknown): Promise<T>;
  /** Rust-owned field bindings by name. */
  fields(names: string[]): any;
  activatePanel(id: string): void;
  report(error: unknown): void;
  /** Extensions: runtime API version and feature detection. */
  api?: ApiInfo;
  /** Extensions: state handed across a reload (see `Hot`). */
  hot?: Hot;
  /** Extensions: commands in the palette and keymap (see `Commands`). */
  commands?: Commands;
  /** Extensions: themes for the whole window (see `Themes`). */
  themes?: Themes;
  /** Register GLSL effects in the native catalog. */
  effects?: import("./effect-definition").Effects;
  /** Layer 4: register GLSL transitions in the native catalog. */
  transitions?: Transitions;
  /** Layer 4: register custom procedural / shader layer types. */
  layers?: Layers;
  /** Extensions: context-menu items (see `Menus`). */
  menus?: Menus;
  /** Extensions: drawing over the viewer (see `Overlays`). */
  overlays?: Overlays;
  /** Extensions: the timeline's columns, metrics, and clip painters. */
  timeline?: Timeline;
  /** Layer 1: contribute items to the native window toolbar. */
  toolbar?: Toolbar;
  statusBar?: StatusBar;
  ui?: Ui;
  /** Layer 1: contribute action buttons/pills to any panel header. */
  panelHeader?: PanelHeader;
  /** Layer 1: contribute items to the macOS menu bar. */
  menuBar?: MenuBar;
  /** Layer 2: register interactive viewer tools & custom gizmos. */
  tools?: Tools;
  /** Layer 3: contribute live UI slots inside built-in panels without forking. */
  slots?: Slots;
  /** Layer 5: subscribe to editor lifecycle events. */
  events?: Events;
  /** Layer 5: inspect and control panels, tools, commands, and notifications. */
  workspace?: Workspace;
  /** Layer 5: native macOS open/save/confirm/alert dialogs. */
  dialogs?: Dialogs;
  /** Layer 6: native filesystem access (including hidden files). */
  fs?: Fs;
  /** Layer 6: persistent per-extension JSON key-value storage across restarts. */
  storage?: Storage;
  /** Layer 6: native HTTP/HTTPS fetch without browser CORS limits. */
  net?: Net;
  /** Layer 6: native system clipboard text access. */
  clipboard?: Clipboard;
  /** Layer 6: native project media import helpers. */
  media?: Media;
  /** Layer 6: Finder reveal, open files/URLs, and command execution. */
  shell?: Shell;
}

/** Context passed to `export function activate(ctx: ActivateContext)`. */
export interface ActivateContext {
  host: HostClient;
  id: string;
  api: ApiInfo;
  hot: Hot;
  commands: Commands;
  themes: Themes;
  effects: import("./effect-definition").Effects;
  transitions: Transitions;
  layers: Layers;
  menus: Menus;
  overlays: Overlays;
  timeline: Timeline;
  toolbar: Toolbar;
  statusBar: StatusBar;
  ui: Ui;
  panelHeader: PanelHeader;
  menuBar: MenuBar;
  tools: Tools;
  slots: Slots;
  events: Events;
  workspace: Workspace;
  dialogs: Dialogs;
  fs: Fs;
  storage: Storage;
  net: Net;
  clipboard: Clipboard;
  media: Media;
  shell: Shell;
}

// ---- Layer 1: Shell & Window Chrome Hooks ------------------------------------

export type ToolbarTarget =
  | "title"
  | "tools"
  | "add"
  | "tasks"
  | "agent"
  | "export"
  | "panelLibrary";

export type ToolbarZone = "leading" | "center" | "after-add" | "trailing";

export interface ToolbarMenuItem {
  id?: string;
  label: string;
  icon?: string;
  key?: string;
  checked?: boolean;
  disabled?: boolean;
  separator?: boolean;
  submenu?: ToolbarMenuItem[];
  run?: () => unknown;
}

export interface ToolbarItem {
  id: string;
  label: string;
  /** SF Symbol name (e.g. "sparkles", "folder", "wand.and.stars"). */
  icon?: string;
  tooltip?: string;
  active?: boolean;
  badge?: string;
  zone?: ToolbarZone;
  order?: number;
  variant?: "default" | "accent" | "subtle";
  disabled?: boolean;
  menu?: ToolbarMenuItem[];
  onClick?: () => unknown;
}

export interface ToolbarPatch {
  hidden?: boolean;
  label?: string;
  icon?: string;
  tooltip?: string;
  badge?: string;
  order?: number;
}

export interface ToolbarAddItem {
  id: string;
  label: string;
  icon?: string;
  section?: string;
  tooltip?: string;
  onClick: () => unknown;
}

export interface Toolbar {
  contribute(item: ToolbarItem): () => void;
  modify(target: ToolbarTarget, patch: ToolbarPatch): () => void;
  contributeAddItem(item: ToolbarAddItem): () => void;
}

export type StatusBarSide = "left" | "center" | "right";
export type StatusBarTarget = "state" | "offline" | "dimensions" | "workspace";

export interface StatusBarItem {
  id: string;
  side: StatusBarSide;
  order?: number;
  text: string;
  icon?: string;
  tooltip?: string;
  tone?: "default" | "accent" | "warning" | "danger";
  clickable?: boolean;
  onClick?: () => unknown;
}

export interface StatusBarPatch {
  hidden?: boolean;
  text?: string;
}

export interface StatusBar {
  contribute(item: StatusBarItem): () => void;
  modify(target: StatusBarTarget, patch: StatusBarPatch): () => void;
}

export interface UiSurfaces {
  toolbarTargets: ToolbarTarget[];
  toolbarZones: ToolbarZone[];
  statusBarTargets: StatusBarTarget[];
  statusBarSides: StatusBarSide[];
  slots: (SlotName | "panel:<panelId>.top" | "panel:<panelId>.bottom")[];
  contextMenus: ContextMenuLocation[];
  replaceablePanels: string[];
  excluded: string[];
}

export interface Ui {
  surfaces(): UiSurfaces;
}

export interface PanelHeaderAction {
  id: string;
  label?: string;
  /** SF Symbol name (e.g. "sparkles", "plus", "slider.horizontal.3"). */
  icon?: string;
  tooltip?: string;
  active?: boolean;
  onClick: () => unknown;
}

export interface PanelHeaderPatch {
  title?: string;
  icon?: string;
  badge?: string;
  hidden?: boolean;
}

export interface PanelHeader {
  /**
   * Add a trailing action button/pill to `panelId` (e.g. "timeline",
   * "inspector", "viewer", "assets", "agent", or "*" for all eligible panels).
   */
  contribute(panelId: string, action: PanelHeaderAction): () => void;
  modify(panelId: string, patch: PanelHeaderPatch): () => void;
}

export type MenuBarItem =
  | "-"
  | {
      label: string;
      key?: string;
      checked?: boolean;
      disabled?: boolean;
      run?: () => unknown;
      submenu?: MenuBarItem[];
    };

export interface MenuBar {
  /**
   * Append items to an existing macOS menu ("File", "Edit", "Composition",
   * "Layer", "Effect", "Playback", "View", "Window") or create a custom
   * top-level menu (e.g. "Scripts", "Tools"). Automatically removed on unload.
   */
  contribute(menuName: string, items: MenuBarItem[]): () => void;
}

// ---- Layer 2: Interactive Viewer Tools & Custom Gizmos -----------------------

export interface ToolModifiers {
  shift: boolean;
  alt: boolean;
  ctrl: boolean;
  meta: boolean;
}

export interface ToolPointerEvent {
  type: "down" | "move" | "up" | "wheel";
  /** Pointer coordinates in viewer body points. */
  x: number;
  y: number;
  /** Pointer coordinates in composition pixel space. */
  compX: number;
  compY: number;
  /** Composition display scale (points per composition pixel). */
  scale: number;
  /** Composition dimensions in pixels. */
  width: number;
  height: number;
  button: number;
  clickCount: number;
  deltaX?: number;
  deltaY?: number;
  modifiers: ToolModifiers;
}

export interface ToolKeyEvent {
  type: "keyDown";
  key: string;
  modifiers: ToolModifiers;
}

export interface ToolDefinition {
  id: string;
  title: string;
  icon?: string;
  shortcut?: string;
  cursor?: string;
  showInToolbar?: boolean;
  /** Optional overlay component rendered above the viewer while this tool is active. */
  overlay?: (props: { frame: Json; active: boolean }) => unknown;
  onActivate?: () => unknown;
  onDeactivate?: () => unknown;
  onPointerDown?: (event: ToolPointerEvent) => unknown;
  onPointerMove?: (event: ToolPointerEvent) => unknown;
  onPointerUp?: (event: ToolPointerEvent) => unknown;
  onWheel?: (event: ToolPointerEvent) => unknown;
  onKeyDown?: (event: ToolKeyEvent) => unknown;
}

export interface Tools {
  register(tool: ToolDefinition): () => void;
  activate(id: string | null): void;
}

// ---- Layer 3: Composable Slots Inside Built-in Panels ------------------------

export type SlotName =
  | "inspector.top"
  | "inspector.bottom"
  | "timeline.toolbar"
  | "timeline.footer"
  | "viewer.hud"
  | "agent.toolbar"
  | "assets.toolbar"
  | "toolbar.leading"
  | "toolbar.center"
  | "toolbar.trailing"
  | "statusBar.left"
  | "statusBar.center"
  | "statusBar.right"
  | "viewer.top"
  | "viewer.bottom"
  | "viewer.controls.left"
  | "viewer.controls.right"
  | "timeline.header"
  | "timeline.top"
  | "timeline.bottom"
  | "inspector.header"
  | "assets.top"
  | "assets.bottom"
  | "effects.top"
  | "effects.bottom"
  | "layerEffects.top"
  | "layerEffects.bottom"
  | "agent.top"
  | "agent.bottom"
  | "easing.top"
  | "easing.bottom"
  | "color.top"
  | "color.bottom"
  | "export.top"
  | "export.bottom"
  | "home.header"
  | "home.top"
  | "home.bottom"
  | "panelLibrary.top"
  | "panelLibrary.bottom"
  | "welcome.top"
  | "welcome.bottom"
  | "store.top"
  | "store.bottom"
  | `panel:${string}.top`
  | `panel:${string}.bottom`;

export type SlotId = SlotName;

export interface Slots {
  /** Render a live native UI component into a built-in panel slot without forking. */
  contribute(slot: SlotId, component: () => unknown): () => void;
  inspector(component: () => unknown, position?: "top" | "bottom"): () => void;
  timelineToolbar(component: () => unknown): () => void;
  timelineFooter(component: () => unknown): () => void;
  viewerHud(component: () => unknown): () => void;
  agentToolbar(component: () => unknown): () => void;
  assetsToolbar(component: () => unknown): () => void;
}

// ---- Layer 4: Custom Transitions & Layer Types -------------------------------

export interface TransitionDefinition {
  id: string;
  label: string;
  group?: string;
  params?: import("./effect-definition").EffectParameter[];
  frag: string;
  rawShader?: boolean;
  [key: string]: unknown;
}

export interface Transitions {
  register(definition: TransitionDefinition): () => void;
  list(): TransitionDefinition[];
  get(id: string): TransitionDefinition | undefined;
}

export interface LayerDefinition {
  id: string;
  label: string;
  icon?: string;
  version?: number;
  defaults?: Record<string, Json>;
  params?: import("./effect-definition").EffectParameter[];
  renderer: {
    kind: "fragment";
    fragment: string;
    [key: string]: unknown;
  };
  [key: string]: unknown;
}

export interface Layers {
  register(definition: LayerDefinition): () => void;
  list(): LayerDefinition[];
  get(id: string): LayerDefinition | undefined;
}

// ---- Layer 5: Lifecycle Events, Workspace Control, & Dialogs -----------------

export type EditorEventName =
  | "playback"
  | "time"
  | "selection"
  | "document"
  | "layout"
  | "tool"
  | "command"
  | "exportStart"
  | "exportComplete";

export interface Events {
  on(event: EditorEventName, handler: (payload: Json) => unknown): () => void;
}

export interface WorkspaceLayout {
  openPanels: string[];
  shownPanels: string[];
  contributedPanels: { id: string; title: string }[];
  tool: string;
}

export interface Workspace {
  openPanel(id: string): Promise<void>;
  closePanel(id: string): Promise<void>;
  focusPanel(id: string): Promise<void>;
  togglePanel(id: string): Promise<void>;
  getLayout(): Promise<WorkspaceLayout>;
  setLayout(layout: WorkspaceLayout | Json): Promise<void>;
  runCommand(name: string, args?: Json[]): Promise<void>;
  notify(message: string): Promise<void>;
  setTool(toolId: string): Promise<void>;
}

export interface OpenDialogOptions {
  title?: string;
  message?: string;
  defaultPath?: string;
  multiple?: boolean;
  showHidden?: boolean;
  extensions?: string[];
  canChooseDirectories?: boolean;
}

export interface SaveDialogOptions {
  title?: string;
  message?: string;
  defaultPath?: string;
  defaultName?: string;
  extensions?: string[];
}

export interface ConfirmDialogOptions {
  title: string;
  message?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  destructive?: boolean;
}

export interface AlertDialogOptions {
  title: string;
  message?: string;
  okLabel?: string;
}

export interface Dialogs {
  openFolder(opts?: OpenDialogOptions): Promise<string[] | null>;
  openFiles(opts?: OpenDialogOptions): Promise<string[] | null>;
  saveFile(opts?: SaveDialogOptions): Promise<string | null>;
  confirm(opts: ConfirmDialogOptions): Promise<boolean>;
  alert(opts: AlertDialogOptions): Promise<void>;
}

// ---- Layer 6: Native Capabilities --------------------------------------------

export interface DirEntry {
  name: string;
  path: string;
  isDir: boolean;
  isFile: boolean;
  isSymlink: boolean;
  isHidden: boolean;
  size: number;
  modifiedMs: number | null;
  extension: string | null;
}

export type FileStat = DirEntry;

export interface ListDirResult {
  path: string;
  parent: string | null;
  entries: DirEntry[];
}

export interface Fs {
  homeDir(): Promise<string>;
  listDir(path?: string, opts?: { includeHidden?: boolean }): Promise<ListDirResult>;
  stat(path: string): Promise<FileStat | null>;
  exists(path: string): Promise<boolean>;
  readText(path: string, opts?: { maxBytes?: number }): Promise<string>;
  readBytes(path: string, opts?: { maxBytes?: number }): Promise<string>;
  writeText(path: string, content: string): Promise<void>;
  writeBytes(path: string, base64: string): Promise<void>;
  mkdir(path: string): Promise<void>;
  rename(from: string, to: string): Promise<void>;
  trash(path: string): Promise<void>;
}

export interface Storage {
  get<T = Json>(key: string): Promise<T | null>;
  set(key: string, value: Json): Promise<void>;
  delete(key: string): Promise<void>;
  keys(): Promise<string[]>;
  getAll(): Promise<Record<string, Json>>;
  clear(): Promise<void>;
}

export interface FetchInit {
  method?: string;
  headers?: Record<string, string>;
  body?: string;
  bodyBase64?: string;
  timeoutMs?: number;
}

export interface FetchResponse {
  ok: boolean;
  status: number;
  statusText: string;
  headers: Record<string, string>;
  text: string;
  bodyBase64: string;
  json<T = Json>(): T;
}

export interface Net {
  fetch(url: string, init?: FetchInit): Promise<FetchResponse>;
}

export interface Clipboard {
  readText(): Promise<string>;
  writeText(text: string): Promise<void>;
}

export interface Media {
  importFiles(
    paths: string[],
    opts?: { folder?: string; addToTimeline?: boolean },
  ): Promise<void>;
}

export interface ExecOptions {
  cwd?: string;
  env?: Record<string, string>;
  stdin?: string;
  timeoutMs?: number;
}

export interface ExecResult {
  code: number;
  stdout: string;
  stderr: string;
  timedOut: boolean;
}

export interface Shell {
  revealInFinder(path: string): Promise<void>;
  openPath(path: string): Promise<boolean>;
  openUrl(url: string): Promise<boolean>;
  exec(program: string, args?: string[], opts?: ExecOptions): Promise<ExecResult>;
}

// ---- Existing v4 Interfaces --------------------------------------------------

export interface Timeline {
  chrome(patch: {
    rowHeight?: number;
    gutterWidth?: number;
    rulerHeight?: number;
    clipRadius?: number;
    keyframeSize?: number;
    showLayerNumbers?: boolean;
    showTypeBadges?: boolean;
    toolbarDensity?: "compact" | "normal" | "comfortable";
    surfaceOrder?: "normal" | "reversed";
  }): void;
  paint(painter: (clips: Json[]) => Record<string, unknown[]>): () => void;
}

export interface Overlays {
  viewer(
    component: (props: { frame: Json }) => unknown,
    options?: { interactive?: boolean },
  ): () => void;
}

export type ContextMenuLocation =
  | "panel:context"
  | "layer:context"
  | "timeline:context"
  | "viewer:context"
  | "assets:context"
  | "inspector:context"
  | "effects:context"
  | "home:context"
  | "toolbar:context"
  | "statusBar:context";

export interface Menus {
  contribute(
    location: ContextMenuLocation | "titlebar:right",
    items: (
      ctx: Record<string, Json>,
    ) => MenuContribution[] | Promise<MenuContribution[]>,
  ): () => void;
}
export type MenuContribution =
  | "-"
  | { header: string }
  | {
      label: string;
      kb?: string | null;
      on?: boolean;
      disabled?: boolean;
      run?: () => unknown;
      submenu?: MenuContribution[];
    };

export interface Themes {
  register(theme: {
    id: string;
    title: string;
    tokens: Record<string, string | number>;
  }): () => void;
  activate(id: string | null): void;
}

export interface Commands {
  register(
    command: { id: string; title: string; key?: string },
    run: (args: Json[]) => unknown,
  ): () => void;
}

export interface Hot {
  data: Record<string, Json>;
  dispose(callback: (data: Record<string, Json>) => void): void;
}

export interface Mounted {
  root: SurfaceRoot;
  dispose(): Promise<void> | void;
  settle?(): Promise<unknown>;
  show?(target?: string): Promise<unknown>;
  publish?(): void;
  refresh?(): Promise<unknown>;
  act?(intent: Json): Promise<unknown>;
}

export type Mount = (env: BuiltinEnv) => Promise<Mounted>;
