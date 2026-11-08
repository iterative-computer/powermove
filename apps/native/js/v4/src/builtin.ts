/**
 * The contract between a panel's `mount` and the window that hosts it. Built-in
 * panels mount through it, and so does a user fork that replaces one: the fork
 * receives exactly the environment the built-in had (`powermove/builtin`).
 */
import type { HostClient } from "./host-api";
import type { Json, Transport } from "./protocol";
import type { SurfaceRoot } from "./renderer";

export interface BuiltinEnv {
  /** The panel id this mount draws (the built-in's id for a fork). */
  panel: string;
  /** Synchronous host operations of this panel's realm. */
  native(operation: string, input?: unknown): any;
  /** An AppKit request answered on the main thread (menus, focus, sheets). */
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
  /** Extensions only: state handed across a reload (see `Hot`). */
  hot?: Hot;
  /** Extensions only: commands in the palette and keymap (see `Commands`). */
  commands?: Commands;
  /** Extensions only: themes for the whole window (see `Themes`). */
  themes?: Themes;
  /** Extensions only: context-menu items (see `Menus`). */
  menus?: Menus;
  /** Extensions only: drawing over the viewer (see `Overlays`). */
  overlays?: Overlays;
  /** Extensions only: the timeline's columns and metrics (see `Timeline`). */
  timeline?: Timeline;
}

/**
 * The timeline's chrome, as v1–3 extensions set `workspace.chrome.timeline`:
 * `chrome({ rowHeight, gutterWidth, rulerHeight, clipRadius, keyframeSize,
 * showLayerNumbers, showTypeBadges, toolbarDensity, surfaceOrder })`, each
 * optional, within v1's ranges. What an extension changes is put back when it
 * leaves.
 */
export interface Timeline {
  chrome(patch: {
    rowHeight?: number;
    gutterWidth?: number;
    rulerHeight?: number;
    clipRadius?: number;
    keyframeSize?: number;
    showLayerNumbers?: boolean;
    showTypeBadges?: boolean;
    toolbarDensity?: "compact" | "normal";
    surfaceOrder?: "normal" | "reversed";
  }): void;
}

/**
 * Drawing over the composition in the viewer, as v1–3 extensions drew on the
 * viewer's overlay canvas: `viewer(Component)` renders `<Component frame />`
 * above the picture and below the transform gizmos, where `frame` is
 * `{ x, y, width, height, scale }`, the composition's rectangle in the
 * overlay's points (`scale` = points per composition pixel). Display only:
 * pointer input goes to the viewer. Returns a function that removes it.
 */
export interface Overlays {
  viewer(component: (props: { frame: Json }) => unknown): () => void;
}

/**
 * Context-menu items, as v1–3's `menus.contribute`: `items(ctx)` runs on every
 * open of that menu (`"panel:context"`, with `ctx.panelId`) and returns items:
 * `"-"`, `{ header }`, or `{ label, kb?, on?, disabled?, run?, submenu? }`. A
 * Promise is waited for at most 100 ms. Returns a function that removes them.
 */
export interface Menus {
  contribute(
    location:
      | "panel:context"
      | "layer:context"
      | "timeline:context"
      | "viewer:context"
      | "titlebar:right",
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

/**
 * Themes an extension offers. Each is listed in the command palette as
 * "Theme: <title>"; choosing it restyles the window's chrome and every panel.
 * `tokens` names design tokens (`"colors.accent"`, `"metrics.gap"`,
 * `"translucency.panel"`, …): colours as `"#rrggbb"`, the rest as numbers.
 * Unknown tokens are refused. `register` returns a function that removes it.
 */
export interface Themes {
  register(theme: {
    id: string;
    title: string;
    tokens: Record<string, string | number>;
  }): () => void;
  /** Shows a registered theme in every window now, as v1–3's
   *  `theme.activate`; `null` puts the built-in look back. */
  activate(id: string | null): void;
}

/**
 * Commands an extension adds to the command palette, optionally with a
 * shortcut (`"cmd+shift+n"`). `run` executes in the extension's own runtime.
 * `register` returns a function that removes the command again.
 */
export interface Commands {
  register(
    command: { id: string; title: string; key?: string },
    run: (args: Json[]) => unknown,
  ): () => void;
}

/**
 * `hot.data` holds what the previous version of this extension left in it
 * from its `hot.dispose` callbacks, which run just before a reload. JSON only.
 */
export interface Hot {
  data: Record<string, Json>;
  dispose(callback: (data: Record<string, Json>) => void): void;
}

export interface Mounted {
  root: SurfaceRoot;
  dispose(): Promise<void> | void;
  /** Resolves when the panel's model has applied what it was sent. */
  settle?(): Promise<unknown>;
  /** The panel was revealed (sheets, Settings). */
  show?(): Promise<unknown>;
  /** A publication wake: pull host-side registries before settling. */
  publish?(): void;
}

export type Mount = (env: BuiltinEnv) => Promise<Mounted>;
