/** Production V8 worker entry. Every built-in owns a separate extension isolate. */
import * as preact from "preact";
import { chunkedSurfaceTransport } from "./surface-transfer";
import { effectsAPI, layersAPI, transitionsAPI } from "../src/effects";
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
  ApiInfo,
  BuiltinEnv,
  Capability,
  Clipboard,
  Commands,
  Dialogs,
  EditorEventName,
  Events,
  Fs,
  Hot,
  Media,
  MenuBar,
  MenuBarItem,
  Menus,
  Mount,
  Mounted,
  Net,
  Overlays,
  PanelHeader,
  PanelHeaderAction,
  PanelHeaderPatch,
  Shell,
  SlotId,
  Slots,
  StatusBar,
  StatusBarItem,
  StatusBarPatch,
  StatusBarTarget,
  Storage,
  Themes,
  Timeline,
  Toolbar,
  ToolbarAddItem,
  ToolbarItem,
  ToolbarMenuItem,
  ToolbarPatch,
  ToolbarTarget,
  ToolDefinition,
  ToolKeyEvent,
  ToolPointerEvent,
  Tools,
  Ui,
  Workspace,
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
import { mount as color } from "../panels/color/mount";
import { mount as easing } from "../panels/easing/mount";
declare const runtime: { engine(operation: string, input?: Json): Json };
const native = (operation: string, input: unknown = null): any =>
  runtime.engine(operation, input as Json);
const report = (error: unknown) => native("report", String(error));

const CAPABILITIES: readonly Capability[] = [
  "commands",
  "themes",
  "menus",
  "overlays",
  "timeline",
  "effects",
  "transitions",
  "layers",
  "toolbar",
  "toolbar.modify",
  "toolbar.contributeAddItem",
  "panelHeader",
  "panelHeader.modify",
  "menuBar",
  "statusBar",
  "statusBar.contribute",
  "statusBar.modify",
  "ui",
  "ui.surfaces",
  "tools",
  "slots",
  "events",
  "workspace",
  "workspace.setLayout",
  "dialogs",
  "fs",
  "storage",
  "net",
  "clipboard",
  "media",
  "shell",
];
const apiInfo: ApiInfo = {
  version: 4,
  capabilities: CAPABILITIES,
  has: (capability: string) =>
    (CAPABILITIES as readonly string[]).includes(capability),
};

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
  color,
  easing,
};
let mounted: Mounted | undefined;
let host: HostClient;

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
function environment(panel: string): BuiltinEnv {
  const identity = native("identity") as HostIdentity;
  host = new HostClient(identity, async (request) => {
    native("host.enqueue", request);
    return reply(() => native("host.poll", request.requestId));
  });
  const rootOptions = { generation: identity.generation };
  const transport: Transport = chunkedSurfaceTransport(native);
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

const hot = {
  data: {} as Record<string, Json>,
  disposers: [] as ((data: Record<string, Json>) => void)[],
};

// ---- Commands, Themes, Effects, Transitions, Layers --------------------------

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
const effectApi = effectsAPI((effects) => {
  native("effects.set", effects);
});
const transitionApi = transitionsAPI((transitions) => {
  native("transitions.set", transitions);
});
const layerApi = layersAPI((layers) => {
  native("layers.set", layers);
});
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
    native("themes.activate", id);
  },
};

// ---- Context Menus -----------------------------------------------------------

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
let menuRuns = new Map<string, () => unknown>();
function isProtectedSettingsSurface(id: string): boolean {
  const lower = id.trim().toLowerCase();
  return (
    lower === "settings" ||
    lower.startsWith("settings.") ||
    lower.startsWith("settings:") ||
    lower === "panel:settings" ||
    lower.startsWith("panel:settings.") ||
    lower.startsWith("panel:settings:")
  );
}
const menuApi: Menus = {
  contribute(location, items) {
    if (typeof location !== "string" || typeof items !== "function")
      throw new Error("A menu contribution needs a location and a function");
    if (isProtectedSettingsSurface(location))
      throw new Error("Extensions cannot modify the Settings UI surface");
    const entry = { location, items };
    menuProviders.push(entry);
    return () => {
      const at = menuProviders.indexOf(entry);
      if (at >= 0) menuProviders.splice(at, 1);
    };
  },
};
async function gatherMenu(location: string, ctx: Record<string, Json>) {
  if (isProtectedSettingsSurface(location)) return [];
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

// ---- Layer 1: Toolbar, Panel Header, Menu Bar --------------------------------

function contributeChrome<K, T>(
  entries: Map<K, T>,
  id: K,
  entry: T,
  publish: () => void,
): () => void {
  const previous = entries.get(id);
  entries.set(id, entry);
  try {
    publish();
  } catch (error) {
    if (previous === undefined) entries.delete(id);
    else entries.set(id, previous);
    throw error;
  }
  return () => {
    if (entries.get(id) !== entry) return;
    entries.delete(id);
    publish();
  };
}

function modifyChrome<K extends string, P extends object>(
  entries: Map<K, P>,
  operation: string,
  targetField: string,
  target: K,
  patch: P,
): () => void {
  return contributeChrome(entries, target, { ...patch }, () =>
    native(
      operation,
      [...entries].map(([id, fields]) => ({ ...fields, [targetField]: id })),
    ),
  );
}

const toolbarItems = new Map<
  string,
  {
    descriptor: object;
    onClick?: () => unknown;
    menuRuns: Map<string, () => unknown>;
  }
>();
const toolbarMods = new Map<ToolbarTarget, ToolbarPatch>();
const toolbarAddItems = new Map<string, ToolbarAddItem>();
let toolbarMenuSerial = 0;

function lowerToolbarItem({ onClick, menu, ...descriptor }: ToolbarItem) {
  const menuRuns = new Map<string, () => unknown>();
  const lower = ({ run, submenu, ...item }: ToolbarMenuItem): object => {
    let actionKey: string | undefined;
    if (typeof run === "function" && !descriptor.disabled && !item.disabled && !item.separator) {
      actionKey = `toolbar-menu-${++toolbarMenuSerial}`;
      menuRuns.set(actionKey, run);
    }
    return { ...item, actionKey, submenu: submenu?.map(lower) };
  };
  return {
    descriptor: {
      ...descriptor,
      icon: descriptor.icon ?? null,
      tooltip: descriptor.tooltip ?? descriptor.label,
      active: !!descriptor.active,
      badge: descriptor.badge ?? null,
      menu: menu?.map(lower),
    },
    onClick: descriptor.disabled ? undefined : onClick,
    menuRuns,
  };
}

const toolbarApi: Toolbar = {
  contribute(item) {
    if (typeof item?.onClick !== "function" && !Array.isArray(item?.menu))
      throw new Error("A toolbar item needs an onClick handler or a menu");
    return contributeChrome(toolbarItems, item.id, lowerToolbarItem(item), () =>
      native(
        "toolbar.set",
        [...toolbarItems.values()].map((item) => item.descriptor),
      ),
    );
  },
  modify(target, patch) {
    return modifyChrome(toolbarMods, "toolbar.modify.set", "target", target, patch);
  },
  contributeAddItem(item) {
    if (typeof item?.onClick !== "function")
      throw new Error("An Add menu item needs an onClick handler");
    return contributeChrome(toolbarAddItems, item.id, { ...item }, () =>
      native(
        "toolbar.addMenu.set",
        [...toolbarAddItems.values()].map(({ onClick, ...item }) => item),
      ),
    );
  },
};

const statusBarItems = new Map<string, StatusBarItem>();
const statusBarMods = new Map<StatusBarTarget, StatusBarPatch>();
const statusBarApi: StatusBar = {
  contribute(item) {
    return contributeChrome(statusBarItems, item.id, { ...item }, () =>
      native(
        "statusBar.set",
        [...statusBarItems.values()].map(({ onClick, ...item }) => ({
          ...item,
          clickable: item.clickable ?? typeof onClick === "function",
        })),
      ),
    );
  },
  modify(target, patch) {
    return modifyChrome(statusBarMods, "statusBar.modify.set", "target", target, patch);
  },
};

const uiApi: Ui = { surfaces: () => native("ui.surfaces") };

const panelHeaderItems = new Map<
  string,
  { panelId: string; action: PanelHeaderAction }
>();
const publishPanelHeaders = () =>
  native(
    "panelHeader.set",
    [...panelHeaderItems.entries()].map(([key, { panelId, action }]) => ({
      key,
      panelId,
      id: action.id,
      label: action.label ?? null,
      icon: action.icon ?? null,
      tooltip: action.tooltip ?? action.label ?? action.id,
      active: !!action.active,
    })),
  );
const panelHeaderMods = new Map<string, PanelHeaderPatch>();
const panelHeaderApi: PanelHeader = {
  contribute(panelId, action) {
    if (typeof panelId !== "string" || !panelId)
      throw new Error("panelHeader.contribute needs a panelId");
    if (!action || typeof action.id !== "string" || !action.id || typeof action.onClick !== "function")
      throw new Error("A panel header action needs an id and an onClick handler");
    const key = `${panelId}:${action.id}`;
    return contributeChrome(
      panelHeaderItems,
      key,
      { panelId, action },
      publishPanelHeaders,
    );
  },
  modify(panelId, patch) {
    return modifyChrome(panelHeaderMods, "panelHeader.modify.set", "panelId", panelId, patch);
  },
};

const menuBarContributions: { menuName: string; items: MenuBarItem[] }[] = [];
let menuBarRuns = new Map<string, () => unknown>();
const publishMenuBar = () => {
  menuBarRuns = new Map();
  let serial = 0;
  const lower = (item: MenuBarItem): Json =>
    item === "-"
      ? "-"
      : {
          label: String(item.label),
          key: item.key ?? null,
          checked: !!item.checked,
          disabled: !!item.disabled,
          actionKey: item.run
            ? (() => {
                const k = `mb${++serial}`;
                menuBarRuns.set(k, item.run!);
                return k;
              })()
            : null,
          submenu: item.submenu ? item.submenu.map(lower) : null,
        };
  native(
    "menuBar.set",
    menuBarContributions.map((c) => ({
      menuName: c.menuName,
      items: c.items.map(lower),
    })),
  );
};
const menuBarApi: MenuBar = {
  contribute(menuName, items) {
    if (typeof menuName !== "string" || !menuName || !Array.isArray(items))
      throw new Error("menuBar.contribute needs a menu name and an items array");
    const entry = { menuName, items };
    menuBarContributions.push(entry);
    publishMenuBar();
    return () => {
      const idx = menuBarContributions.indexOf(entry);
      if (idx >= 0) {
        menuBarContributions.splice(idx, 1);
        publishMenuBar();
      }
    };
  },
};

// ---- Layer 2: Viewer Overlays & Interactive Tools ----------------------------

let overlay: { root: any; component: any; interactive: boolean } | null = null;
let viewerFrame: Json = { x: 0, y: 0, width: 0, height: 0, scale: 1 };
let activeToolId: string | null = null;
const registeredTools = new Map<string, ToolDefinition>();
let toolOverlay: { root: any; toolId: string } | null = null;
let envRootOptions: any = { generation: 1 };

async function renderOverlay() {
  if (overlay) {
    const Overlay = overlay.component;
    await overlay.root.render(<Overlay frame={viewerFrame} />);
    await overlay.root.flush();
  }
  if (toolOverlay && activeToolId === toolOverlay.toolId) {
    const tool = registeredTools.get(toolOverlay.toolId);
    if (tool?.overlay) {
      const ToolOverlay = tool.overlay;
      await toolOverlay.root.render(<ToolOverlay frame={viewerFrame} active={true} />);
      await toolOverlay.root.flush();
    }
  }
}

function overlaysFor(env: { rootOptions: any }): Overlays {
  return {
    viewer(component, options) {
      if (typeof component !== "function")
        throw new Error("A viewer overlay is a component");
      if (overlay) {
        const old = overlay;
        overlay = null;
        void old.root.dispose().catch(() => {});
        native("overlay.clear", null);
      }
      const interactive = !!options?.interactive;
      const entry: { root: any; component: any; interactive: boolean } = {
        root: null,
        component,
        interactive,
      };
      const transport: Transport = async (commit) =>
        overlay === entry
          ? native("overlay.commit", { ...commit, interactive })
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

const publishTools = () =>
  native(
    "tools.set",
    [...registeredTools.values()].map((t) => ({
      id: t.id,
      title: t.title,
      icon: t.icon ?? null,
      shortcut: t.shortcut ?? null,
      cursor: t.cursor ?? null,
      showInToolbar: t.showInToolbar,
    })),
  );

function syncToolOverlay() {
  if (toolOverlay && toolOverlay.toolId !== activeToolId) {
    const old = toolOverlay;
    toolOverlay = null;
    void old.root.dispose().catch(() => {});
    native("overlay.clear", { tool: true });
  }
  if (activeToolId && !toolOverlay) {
    const tool = registeredTools.get(activeToolId);
    if (tool?.overlay) {
      const id = activeToolId;
      const entry: { root: any; toolId: string } = { root: null, toolId: id };
      const transport: Transport = async (commit) =>
        toolOverlay === entry
          ? native("overlay.commit", { ...commit, interactive: false })
          : { accepted: false, revision: -1, error: "tool overlay replaced" };
      entry.root = createRoot(transport, report, envRootOptions);
      toolOverlay = entry;
      void renderOverlay().catch(report);
    }
  }
}

const toolsApi: Tools = {
  register(tool) {
    if (!tool || typeof tool.id !== "string" || !tool.id || typeof tool.title !== "string")
      throw new Error("A tool needs an id and a title");
    registeredTools.set(tool.id, tool);
    publishTools();
    return () => {
      if (registeredTools.get(tool.id) === tool) {
        if (activeToolId === tool.id) {
          activeToolId = null;
          syncToolOverlay();
        }
        registeredTools.delete(tool.id);
        publishTools();
      }
    };
  },
  activate(id) {
    void platform("ext.workspace.setTool", { tool: id }).catch(report);
  },
};

// ---- Layer 3: Composable Built-in Panel Slots --------------------------------

const slotRoots = new Map<SlotId, { root: any; component: () => unknown }>();
function slotsFor(env: { rootOptions: any }): Slots {
  const contribute = (slot: SlotId, component: () => unknown) => {
    if (typeof slot !== "string" || !slot || typeof component !== "function")
      throw new Error("slots.contribute needs a slot id and a component");
    native("slot.validate", { slot });
    const prev = slotRoots.get(slot);
    if (prev) {
      slotRoots.delete(slot);
      void prev.root.dispose().catch(() => {});
      native("slot.clear", { slot });
    }
    const entry: { root: any; component: () => unknown } = {
      root: null,
      component,
    };
    const transport: Transport = async (commit) =>
      slotRoots.get(slot) === entry
        ? native("slot.commit", { slot, commit })
        : { accepted: false, revision: -1, error: "slot replaced" };
    entry.root = createRoot(transport, report, env.rootOptions);
    slotRoots.set(slot, entry);
    const Comp = component;
    void entry.root
      .render(<Comp />)
      .then(() => entry.root.flush())
      .catch(report);
    return () => {
      if (slotRoots.get(slot) !== entry) return;
      slotRoots.delete(slot);
      void entry.root.dispose().catch(() => {});
      native("slot.clear", { slot });
    };
  };
  return {
    contribute,
    inspector: (c, position = "bottom") =>
      contribute(position === "top" ? "inspector.top" : "inspector.bottom", c),
    timelineToolbar: (c) => contribute("timeline.toolbar", c),
    timelineFooter: (c) => contribute("timeline.footer", c),
    viewerHud: (c) => contribute("viewer.hud", c),
    agentToolbar: (c) => contribute("agent.toolbar", c),
    assetsToolbar: (c) => contribute("assets.toolbar", c),
  };
}

// ---- Layer 5: Lifecycle Events, Workspace, Dialogs ---------------------------

const eventListeners = new Map<
  EditorEventName,
  Set<(payload: Json) => unknown>
>();
const syncEventSubscriptions = () => {
  const active = [...eventListeners.entries()]
    .filter(([, set]) => set.size > 0)
    .map(([name]) => name);
  native("events.subscribe", active);
};
const eventsApi: Events = {
  on(event, handler) {
    if (typeof event !== "string" || typeof handler !== "function")
      throw new Error("events.on needs an event name and a handler");
    let set = eventListeners.get(event);
    if (!set) {
      set = new Set();
      eventListeners.set(event, set);
    }
    set.add(handler);
    syncEventSubscriptions();
    return () => {
      set!.delete(handler);
      if (set!.size === 0) eventListeners.delete(event);
      syncEventSubscriptions();
    };
  },
};

const workspaceApi: Workspace = {
  openPanel: (id) => platform("ext.workspace.openPanel", { id }),
  closePanel: (id) => platform("ext.workspace.closePanel", { id }),
  focusPanel: (id) => platform("ext.workspace.focusPanel", { id }),
  togglePanel: (id) => platform("ext.workspace.togglePanel", { id }),
  getLayout: () => platform("ext.workspace.getLayout", {}),
  setLayout: (layout) =>
    platform("ext.workspace.runCommand", { name: "workspaceSetLayout", args: [layout] }),
  runCommand: (name, args = []) =>
    platform("ext.workspace.runCommand", { name, args }),
  notify: (message) => platform("ext.workspace.notify", { message }),
  setTool: (tool) => platform("ext.workspace.setTool", { tool }),
};

const dialogsApi: Dialogs = {
  openFolder: (opts = {}) => platform("ext.dialogs.openFolder", opts),
  openFiles: (opts = {}) => platform("ext.dialogs.openFiles", opts),
  saveFile: (opts = {}) => platform("ext.dialogs.saveFile", opts),
  confirm: (opts) => platform("ext.dialogs.confirm", opts),
  alert: (opts) => platform("ext.dialogs.alert", opts),
};

// ---- Layer 6: Native Capabilities (fs, storage, net, clipboard, media, shell)

const fsApi: Fs = {
  homeDir: () => platform("ext.fs.homeDir", {}),
  listDir: (path = "~", opts = {}) =>
    platform("ext.fs.listDir", { path, includeHidden: opts.includeHidden ?? true }),
  stat: (path) => platform("ext.fs.stat", { path }),
  exists: (path) => platform("ext.fs.exists", { path }),
  readText: (path, opts = {}) => platform("ext.fs.readText", { path, ...opts }),
  readBytes: (path, opts = {}) => platform("ext.fs.readBytes", { path, ...opts }),
  writeText: (path, content) => platform("ext.fs.writeText", { path, content }),
  writeBytes: (path, base64) => platform("ext.fs.writeBytes", { path, base64 }),
  mkdir: (path) => platform("ext.fs.mkdir", { path }),
  rename: (from, to) => platform("ext.fs.rename", { from, to }),
  trash: (path) => platform("ext.fs.trash", { path }),
};

const storageApi: Storage = {
  get: (key) => platform("ext.storage.get", { key }),
  set: (key, value) => platform("ext.storage.set", { key, value }),
  delete: (key) => platform("ext.storage.delete", { key }),
  keys: () => platform("ext.storage.keys", {}),
  getAll: () => platform("ext.storage.getAll", {}),
  clear: () => platform("ext.storage.clear", {}),
};

const netApi: Net = {
  async fetch(url, init = {}) {
    const res = await platform("ext.net.fetch", { url, ...init });
    return {
      ...res,
      json<T = Json>() {
        return JSON.parse(res.text) as T;
      },
    };
  },
};

const clipboardApi: Clipboard = {
  readText: () => platform("ext.clipboard.readText", {}),
  writeText: (text) => platform("ext.clipboard.writeText", { text }),
};

const mediaApi: Media = {
  importFiles: (paths, opts = {}) =>
    platform("ext.media.importFiles", { paths, ...opts }),
};

const shellApi: Shell = {
  revealInFinder: (path) => platform("ext.shell.revealInFinder", { path }),
  openPath: (path) => platform("ext.shell.openPath", { path }),
  openUrl: (url) => platform("ext.shell.openUrl", { url }),
  exec: (program, args = [], opts = {}) =>
    platform("ext.shell.exec", { program, args, ...opts }),
};

// ---- Timeline Chrome & Clip Painter ------------------------------------------

let clipPainter: ((clips: Json[]) => unknown) | null = null;
const timelineApi: Timeline = {
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

// ---- Extension Mount ---------------------------------------------------------

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
  const baseEnv = environment(replaces ?? id);
  envRootOptions = baseEnv.rootOptions;
  const overlays = overlaysFor(baseEnv);
  const slots = slotsFor(baseEnv);
  const env: BuiltinEnv = {
    ...baseEnv,
    api: apiInfo,
    hot: api,
    commands: commandApi,
    themes: themeApi,
    effects: effectApi,
    transitions: transitionApi,
    layers: layerApi,
    menus: menuApi,
    overlays,
    timeline: timelineApi,
    toolbar: toolbarApi,
    statusBar: statusBarApi,
    ui: uiApi,
    panelHeader: panelHeaderApi,
    menuBar: menuBarApi,
    tools: toolsApi,
    slots,
    events: eventsApi,
    workspace: workspaceApi,
    dialogs: dialogsApi,
    fs: fsApi,
    storage: storageApi,
    net: netApi,
    clipboard: clipboardApi,
    media: mediaApi,
    shell: shellApi,
  };
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
  if (
    typeof namespace.view !== "function" &&
    typeof namespace.activate !== "function"
  ) {
    throw new Error("A v4 extension exports view(), mount(env), or activate(ctx)");
  }
  const root = env.root();
  await namespace.activate?.({
    host,
    id,
    api: apiInfo,
    hot: api,
    commands: commandApi,
    themes: themeApi,
    effects: effectApi,
    transitions: transitionApi,
    layers: layerApi,
    menus: menuApi,
    overlays,
    timeline: timelineApi,
    toolbar: toolbarApi,
    statusBar: statusBarApi,
    ui: uiApi,
    panelHeader: panelHeaderApi,
    menuBar: menuBarApi,
    tools: toolsApi,
    slots,
    events: eventsApi,
    workspace: workspaceApi,
    dialogs: dialogsApi,
    fs: fsApi,
    storage: storageApi,
    net: netApi,
    clipboard: clipboardApi,
    media: mediaApi,
    shell: shellApi,
  });
  if (typeof namespace.view === "function") {
    const View = namespace.view;
    await root.render(<View />);
    await root.flush();
  }
  mounted = { root, dispose: () => root.dispose() };
  return true;
}

(globalThis as any).__pmBuiltin = {
  mount,
  async mountExtension(...args: Parameters<typeof mountExtension>) {
    try {
      return await mountExtension(...args);
    } catch (error) {
      // An ordinary activation exception can leave the V8 realm alive.
      try {
        native("extension.activationFailed");
      } catch {
        // Preserve the original error if the realm can no longer call the host.
      }
      throw error;
    }
  },
  hot() {
    for (const dispose of hot.disposers) dispose(hot.data);
    return JSON.stringify(hot.data);
  },
  async publish(refresh = false) {
    for (const p of native("host.publications") as HostPublication[])
      host.publish(p);
    mounted?.publish?.();
    if (refresh) await mounted?.refresh?.();
    await settle();
    return true;
  },
  async event(event: NativeEvent) {
    if (!mounted) throw new Error("Panel not mounted");
    await mounted.root.dispatchNative(event);
    await settle();
    return true;
  },
  async overlayEvent(event: NativeEvent) {
    if (overlay) {
      await overlay.root.dispatchNative(event);
      await overlay.root.flush();
    }
    return true;
  },
  async slotEvent(slot: SlotId, event: NativeEvent) {
    const entry = slotRoots.get(slot);
    if (entry) {
      await entry.root.dispatchNative(event);
      await entry.root.flush();
    }
    return true;
  },
  async command(name: string, args: Json) {
    await host.command(name, args);
    await mounted?.refresh?.();
    await this.publish();
    return true;
  },
  paintClips(clips: Json[]) {
    return JSON.stringify(clipPainter ? (clipPainter(clips) ?? {}) : null);
  },
  viewerFrame(frame: Json) {
    if (JSON.stringify(frame) === JSON.stringify(viewerFrame))
      return Promise.resolve(true);
    viewerFrame = frame;
    return renderOverlay().then(() => true);
  },
  gatherMenu(location: string, ctx: Record<string, Json>) {
    return gatherMenu(location, ctx);
  },
  async runMenu(key: string) {
    const run = menuRuns.get(key);
    if (!run) throw new Error("This menu item is gone");
    await run();
    await settle();
    return true;
  },
  async runCommand(id: string, args: Json[]) {
    const command = commands.get(id);
    if (!command) throw new Error(`No command ${id}`);
    await command.run(args);
    await settle();
    return true;
  },
  async runToolbar(id: string) {
    const item = toolbarItems.get(id);
    if (!item) throw new Error(`No toolbar item ${id}`);
    await item.onClick?.();
    await settle();
    return true;
  },
  async runToolbarMenu(key: string) {
    for (const { menuRuns } of toolbarItems.values()) {
      const run = menuRuns.get(key);
      if (!run) continue;
      await run();
      await settle();
      return true;
    }
    throw new Error("This toolbar menu item is gone");
  },
  async runToolbarAddItem(id: string) {
    const item = toolbarAddItems.get(id);
    if (!item) throw new Error(`No Add menu item ${id}`);
    await item.onClick();
    await settle();
    return true;
  },
  async runStatusBar(id: string) {
    const item = statusBarItems.get(id);
    if (!item) throw new Error(`No status bar item ${id}`);
    if (item.clickable !== false) await item.onClick?.();
    await settle();
    return true;
  },
  async runPanelHeader(key: string) {
    const item = panelHeaderItems.get(key);
    if (!item) throw new Error(`No panel header item ${key}`);
    await item.action.onClick();
    await settle();
    return true;
  },
  async runMenuBar(key: string) {
    const run = menuBarRuns.get(key);
    if (!run) throw new Error("This menu bar item is gone");
    await run();
    await settle();
    return true;
  },
  async toolState(id: string | null) {
    if (activeToolId === id) return true;
    const prev = activeToolId ? registeredTools.get(activeToolId) : undefined;
    activeToolId = id && registeredTools.has(id) ? id : null;
    if (prev?.onDeactivate) await prev.onDeactivate();
    const next = activeToolId ? registeredTools.get(activeToolId) : undefined;
    if (next?.onActivate) await next.onActivate();
    syncToolOverlay();
    await settle();
    return true;
  },
  async toolEvent(id: string, event: ToolPointerEvent | ToolKeyEvent) {
    const tool = registeredTools.get(id);
    if (!tool) return false;
    switch (event.type) {
      case "down":
        await tool.onPointerDown?.(event);
        break;
      case "move":
        await tool.onPointerMove?.(event);
        break;
      case "up":
        await tool.onPointerUp?.(event);
        break;
      case "wheel":
        await tool.onWheel?.(event);
        break;
      case "keyDown":
        await tool.onKeyDown?.(event);
        break;
    }
    await settle();
    return true;
  },
  async emitEvent(name: EditorEventName, payload: Json) {
    const listeners = eventListeners.get(name);
    if (!listeners || listeners.size === 0) return true;
    for (const listener of [...listeners]) {
      try {
        await listener(payload);
      } catch (err) {
        report(err);
      }
    }
    await settle();
    return true;
  },
  async write(handle: string, value: Json) {
    await host.command("panel.binding", { handle, value });
    await this.publish();
    return true;
  },
  async act(intent: Json) {
    await mounted?.act?.(intent);
    await settle();
    return true;
  },
  async show(target?: string) {
    await mounted?.show?.(target);
    await settle();
    return true;
  },
  async dispose() {
    for (const entry of slotRoots.values()) {
      await entry.root.dispose().catch(() => {});
    }
    slotRoots.clear();
    await mounted?.dispose();
    host?.retire();
    native("host.retire");
    return true;
  },
};
async function settle() {
  await mounted?.settle?.();
  await mounted?.root.flush();
  for (const entry of slotRoots.values()) {
    await entry.root.flush();
  }
  if (overlay) await overlay.root.flush();
  if (toolOverlay) await toolOverlay.root.flush();
}
