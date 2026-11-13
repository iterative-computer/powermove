/** Production V8 worker entry. Every built-in owns a separate extension isolate. */
import * as preact from "preact";
import * as preactHooks from "preact/hooks";
import * as runtimeModule from "../src";
import { createRoot } from "../src";
import * as ui from "../src/ui";
import * as hooks from "../src/hooks";
import * as jsxRuntime from "../src/jsx-runtime";
import * as protocol from "../src/protocol";
import * as hostApi from "../src/host-api";
import * as components from "../src/components";
import * as memo from "../src/memo";
import * as renderer from "../src/renderer";
import {
  HostClient,
  type HostIdentity,
  type HostPublication,
} from "../src/host-api";
import { type Json, type NativeEvent, type Transport } from "../src/protocol";
import type {
  BuiltinEnv,
  Commands,
  Hot,
  Mount,
  Mounted,
  Themes,
  Menus,
  Overlays,
  Timeline,
} from "../src/builtin";
import { mount as layerEffects } from "../panels/effect-controls/mount";
import { mount as fxbrowser } from "../panels/effects/mount";
import { mount as timeline } from "../panels/timeline/mount";
import { mount as inspector } from "../panels/inspector/mount";
import { mount as viewer } from "../panels/viewer/mount";
import { mount as assets } from "../panels/project/mount";
import { mount as agent } from "../panels/agent/mount";
import { mount as settings } from "../panels/settings/mount";
import { mount as exportPanel } from "../panels/export/mount";
declare const runtime: { engine(operation: string, input?: Json): Json };
const native = (operation: string, input: unknown = null): any =>
  runtime.engine(operation, input as Json);
const report = (error: unknown) => native("report", String(error));
/** Each built-in panel (`panels.json`) to the module that mounts it. */
const MOUNTS: Record<string, Mount> = {
  "layer-effects": layerEffects,
  fxbrowser,
  timeline,
  inspector,
  viewer,
  assets,
  agent,
  settings,
  export: exportPanel,
};
let mounted: Mounted | undefined;
let host: HostClient;
/** A native reply: polled at once while it is quick, then backing off to
 * once a frame, so panels waiting on a busy kernel don't spin (each poll
 * also advances the panel's backend). */
async function reply<T>(read: () => T | null | undefined): Promise<T> {
  for (let attempt = 0; ; attempt++) {
    const value = read();
    if (value) return value;
    const delay = attempt < 4 ? 1 : Math.min(16, 2 ** (attempt - 3));
    await new Promise((resolve) => setTimeout(resolve, delay));
  }
}
async function platform(operation: string, input: unknown = null) {
  const id = native("platform.enqueue", { operation, input });
  const result = await reply(() => native("platform.poll", id));
  if (!result.accepted) throw new Error(result.error);
  return result.value;
}
/** The realm's host connection and everything a panel mount may use. */
function environment(panel: string): BuiltinEnv {
  const identity = native("identity") as HostIdentity;
  host = new HostClient(identity, async (request) => {
    native("host.enqueue", request);
    return reply(() => native("host.poll", request.requestId));
  });
  const rootOptions = { generation: identity.generation };
  const transport: Transport = async (commit) =>
    native("surface.commit", commit);
  return {
    panel,
    native,
    platform,
    host,
    transport,
    rootOptions,
    root: () => createRoot(transport, report, rootOptions),
    command: async <T = Json | undefined,>(name: string, args: unknown = {}) =>
      (await host.command(name, args as Json)).value as T,
    fields: (names: string[]) => native("fields.register", names),
    activatePanel: (id: string) => {
      void platform("activatePanel", id).catch(report);
    },
    report,
  };
}
async function mount(panel: string) {
  if (mounted) throw new Error("Panel already mounted");
  const entry = MOUNTS[panel];
  if (!entry) throw new Error(`Unregistered built-in panel: ${panel}`);
  mounted = await entry(environment(panel));
  return true;
}
declare const __linkModules: (graph: string) => [Promise<unknown>, any];
/** What an extension may import, by specifier. Built-in panel sources use the
 * same names, so a fork of one compiles against the modules the panel uses. */
const RUNTIME: Record<string, any> = {
  preact,
  "preact/hooks": preactHooks,
  "powermove/ui": ui,
  "powermove/hooks": hooks,
  "powermove/jsx-runtime": jsxRuntime,
  "powermove/jsx-dev-runtime": jsxRuntime,
  "powermove/runtime": runtimeModule,
  "powermove/protocol": protocol,
  "powermove/host-api": hostApi,
  "powermove/components": components,
  "powermove/memo": memo,
  "powermove/renderer": renderer,
  "powermove/builtin": {},
};
/**
 * A v4 extension. It exports `view()` (a component, drawn as its own panel) or
 * `mount(env)`, which gets the same environment as a built-in panel. With
 * `replaces`, it is a fork drawing that built-in's panel in its place.
 */
/** `api.hot`: data an extension hands to its next version when it reloads. */
const hot = {
  data: {} as Record<string, Json>,
  disposers: [] as ((data: Record<string, Json>) => void)[],
};
/** Commands this realm added; the window lists them in its palette and keymap. */
const commands = new Map<
  string,
  { title: string; key?: string; run: (args: Json[]) => unknown }
>();
const publishCommands = () =>
  native(
    "commands.set",
    [...commands].map(([id, c]) => ({
      id,
      title: c.title,
      key: c.key ?? null,
    })),
  );
const commandApi: Commands = {
  register({ id, title, key }, run) {
    if (typeof id !== "string" || !id || typeof title !== "string" || !title)
      throw new Error("A command needs an id and a title");
    commands.set(id, { title, key, run });
    publishCommands();
    return () => {
      if (commands.get(id)?.run === run) {
        commands.delete(id);
        publishCommands();
      }
    };
  },
};
/** Themes this realm offers; the window lists them in its palette. */
const themes = new Map<
  string,
  { title: string; tokens: Record<string, string | number> }
>();
const publishThemes = () =>
  native(
    "themes.set",
    [...themes].map(([id, t]) => ({ id, title: t.title, tokens: t.tokens })),
  );
const themeApi: Themes = {
  register(theme) {
    const { id, title, tokens } = theme ?? ({} as never);
    if (typeof id !== "string" || !id || typeof title !== "string" || !title)
      throw new Error("A theme needs an id and a title");
    if (!tokens || typeof tokens !== "object" || Array.isArray(tokens))
      throw new Error("A theme needs a tokens object");
    const entry = { title, tokens: { ...tokens } };
    themes.set(id, entry);
    publishThemes();
    return () => {
      if (themes.get(id) === entry) {
        themes.delete(id);
        publishThemes();
      }
    };
  },
  activate(id) {
    if (id !== null && !themes.has(id))
      throw new Error(`No theme ${String(id)} is registered`);
    // The window shows the latest activation from any extension.
    native("themes.activate", id);
  },
};
/** Context-menu items this realm adds (main's `MenusAPI.contribute`). */
type MenuItem =
  | "-"
  | { header: string }
  | {
      label: string;
      kb?: string | null;
      on?: boolean;
      disabled?: boolean;
      run?: () => unknown;
      submenu?: MenuItem[];
    };
const menuProviders: {
  location: string;
  items: (ctx: Record<string, Json>) => MenuItem[] | Promise<MenuItem[]>;
}[] = [];
/** The runnable items of the menu last gathered, by key. */
let menuRuns = new Map<string, () => unknown>();
const menuApi: Menus = {
  contribute(location, items) {
    if (typeof location !== "string" || typeof items !== "function")
      throw new Error("A menu contribution needs a location and a function");
    const entry = { location, items };
    menuProviders.push(entry);
    return () => {
      const at = menuProviders.indexOf(entry);
      if (at >= 0) menuProviders.splice(at, 1);
    };
  },
};
/** As main: a Promise is waited for at most 100 ms per open. */
async function gatherMenu(location: string, ctx: Record<string, Json>) {
  menuRuns = new Map();
  let serial = 0;
  const lower = (item: MenuItem): Json =>
    item === "-"
      ? "-"
      : "header" in item
        ? { header: String(item.header) }
        : {
            label: String(item.label),
            kb: item.kb ?? null,
            on: !!item.on,
            disabled: !!item.disabled,
            key: item.run
              ? (() => {
                  const key = `m${++serial}`;
                  menuRuns.set(key, item.run!);
                  return key;
                })()
              : null,
            submenu: item.submenu ? item.submenu.map(lower) : null,
          };
  const out: Json[] = [];
  for (const provider of menuProviders.filter((p) => p.location === location)) {
    const items = await Promise.race([
      Promise.resolve(provider.items(ctx)).catch(() => [] as MenuItem[]),
      new Promise<MenuItem[]>((resolve) => setTimeout(() => resolve([]), 100)),
    ]);
    for (const item of Array.isArray(items) ? items : []) out.push(lower(item));
  }
  return JSON.stringify(out);
}
/** This realm's viewer overlay (`overlays.viewer`): display only, drawn over
 * the composition; `frame` is where the composition sits, in its points. */
let overlay: { root: any; component: any } | null = null;
let viewerFrame: Json = { x: 0, y: 0, width: 0, height: 0, scale: 1 };
async function renderOverlay() {
  if (!overlay) return;
  const Overlay = overlay.component;
  await overlay.root.render(<Overlay frame={viewerFrame} />);
  await overlay.root.flush();
}
function overlaysFor(env: { rootOptions: any }): Overlays {
  return {
    viewer(component) {
      if (typeof component !== "function")
        throw new Error("A viewer overlay is a component");
      if (overlay) {
        // A replacement starts from its own first commit; the old root's
        // teardown commits are dropped so they can't land on the new one.
        const old = overlay;
        overlay = null;
        void old.root.dispose().catch(() => {});
        native("overlay.clear", null);
      }
      const entry: { root: any; component: any } = { root: null, component };
      const transport: Transport = async (commit) =>
        overlay === entry
          ? native("overlay.commit", commit)
          : { accepted: false, revision: -1, error: "overlay replaced" };
      entry.root = createRoot(transport, report, env.rootOptions);
      overlay = entry;
      void renderOverlay().catch(report);
      return () => {
        if (overlay !== entry) return;
        overlay = null;
        void entry.root.dispose().catch(() => {});
        native("overlay.clear", null);
      };
    },
  };
}
/** This realm's clip painter (`timeline.paint`), run when clips move. */
let clipPainter: ((clips: Json[]) => unknown) | null = null;
const timelineApi: Timeline = {
  /** v1–3's timeline chrome (`workspace.chrome.timeline`) for v4. */
  chrome(patch) {
    if (!patch || typeof patch !== "object")
      throw new Error("Timeline chrome is an object of fields");
    native("timeline.chrome", patch as Json);
  },
  paint(painter) {
    if (typeof painter !== "function")
      throw new Error("A clip painter is a function");
    clipPainter = painter as (clips: Json[]) => unknown;
    native("timeline.painted", true);
    return () => {
      if (clipPainter === painter) {
        clipPainter = null;
        native("timeline.painted", false);
      }
    };
  },
};
async function mountExtension(
  id: string,
  text: string,
  replaces?: string,
  hotData?: string,
) {
  if (mounted) throw new Error("Panel already mounted");
  hot.data = hotData ? JSON.parse(hotData) : {};
  const api: Hot = {
    data: hot.data,
    dispose: (callback) => {
      hot.disposers.push(callback);
    },
  };
  const env = {
    ...environment(replaces ?? id),
    hot: api,
    commands: commandApi,
    themes: themeApi,
    menus: menuApi,
  };
  (env as any).overlays = overlaysFor(env as any);
  (env as any).timeline = timelineApi;
  (globalThis as any).__powermove_runtime = RUNTIME;
  const graph = JSON.parse(text);
  const url = (name: string) =>
    name.startsWith("runtime:")
      ? `pmruntime://${name.slice(8)}`
      : `pmext://${id}/${name}`;
  const sources: Record<string, string> = {};
  const edges: Record<string, Record<string, string>> = {};
  for (const [name, source] of Object.entries(graph.sources))
    sources[url(name)] = source as string;
  for (const [name, targets] of Object.entries(graph.edges)) {
    edges[url(name)] = {};
    for (const [specifier, target] of Object.entries(
      targets as Record<string, string>,
    )) {
      edges[url(name)]![specifier] = url(target);
      if (target.startsWith("runtime:")) {
        const module = target.slice(8);
        if (!(module in RUNTIME))
          throw new Error(`Unsupported import: ${module}`);
        sources[url(target)] = [
          `const m = globalThis.__powermove_runtime[${JSON.stringify(module)}];`,
          ...Object.keys(RUNTIME[module])
            .filter((key) => key !== "default")
            .map((key) => `export const ${key} = m.${key};`),
          "export default m;",
        ].join("\n");
      }
    }
  }
  const [done, namespace] = __linkModules(
    JSON.stringify({ sources, edges, entry: url("powermove:bundle") }),
  );
  await done;
  if (typeof namespace.mount === "function") {
    mounted = await namespace.mount(env);
    return true;
  }
  if (typeof namespace.view !== "function")
    throw new Error("A v4 extension exports view() or mount(env)");
  const root = env.root();
  await namespace.activate?.({
    host,
    id,
    hot: api,
    commands: commandApi,
    themes: themeApi,
    menus: menuApi,
    overlays: (env as any).overlays,
    timeline: timelineApi,
  });
  // A component, so the view's hooks have one to belong to.
  const View = namespace.view;
  await root.render(<View />);
  await root.flush();
  mounted = { root, dispose: () => root.dispose() };
  return true;
}
(globalThis as any).__pmBuiltin = {
  mount,
  mountExtension,
  /** Before a reload: the extension's `hot.dispose` callbacks fill `hot.data`. */
  hot() {
    for (const dispose of hot.disposers) dispose(hot.data);
    return JSON.stringify(hot.data);
  },
  async publish() {
    for (const p of native("host.publications") as HostPublication[])
      host.publish(p);
    mounted?.publish?.();
    await settle();
    return true;
  },
  async event(event: NativeEvent) {
    if (!mounted) throw new Error("Panel not mounted");
    await mounted.root.dispatchNative(event);
    await settle();
    return true;
  },
  async command(name: string, args: Json) {
    await host.command(name, args);
    await mounted?.refresh?.();
    await this.publish();
    return true;
  },
  /** The timeline shows these clips: this realm's decorations for them. */
  paintClips(clips: Json[]) {
    return JSON.stringify(clipPainter ? (clipPainter(clips) ?? {}) : null);
  },
  /** Where the composition sits in the viewer (for the overlay). */
  viewerFrame(frame: Json) {
    if (JSON.stringify(frame) === JSON.stringify(viewerFrame))
      return Promise.resolve(true);
    viewerFrame = frame;
    return renderOverlay().then(() => true);
  },
  /** The window is opening a menu at `location`: this realm's items. */
  gatherMenu(location: string, ctx: Record<string, Json>) {
    return gatherMenu(location, ctx);
  },
  /** The person chose one of this realm's menu items. */
  async runMenu(key: string) {
    const run = menuRuns.get(key);
    if (!run) throw new Error("This menu item is gone");
    await run();
    await settle();
    return true;
  },
  /** The palette or a shortcut ran a command this extension registered. */
  async runCommand(id: string, args: Json[]) {
    const command = commands.get(id);
    if (!command) throw new Error(`No command ${id}`);
    await command.run(args);
    await settle();
    return true;
  },
  async write(handle: string, value: Json) {
    await host.command("panel.binding", { handle, value });
    await this.publish();
    return true;
  },
  async show(target?: string) {
    await mounted?.show?.(target);
    await settle();
    return true;
  },
  async dispose() {
    await mounted?.dispose();
    host?.retire();
    native("host.retire");
    return true;
  },
};
async function settle() {
  await mounted?.settle?.();
  await mounted?.root.flush();
}
