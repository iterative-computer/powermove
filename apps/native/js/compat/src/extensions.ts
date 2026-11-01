import { createExtensionAPI } from "./frozen/host/host";
import { HostError } from "./channel";
import { makePMFacades, mergeDescriptors } from "./facades";
import * as bindings from "./frozen/host/binding";
import type { Data } from "./types";
import type { CompatRealm } from "./realm";

export interface ExtensionOptions {
  manifest?: Data;
  vars?: Readonly<Record<string, string>>;
  scope?: "builtin" | "user" | "store";
}
export interface ExtensionHandle {
  api: Data;
  setActivating(on: boolean): void;
  dispose(): void;
}
/** A complete declared namespace whose operations require the native backend. */
function required(family: string, members: string[]): Data {
  return Object.fromEntries(
    members.map((member) => [
      member,
      () => {
        throw new HostError(
          "Unavailable",
          `${family}.${member} backend is not attached`,
        );
      },
    ]),
  );
}
export function extensionAPI(
  realm: CompatRealm,
  id: string,
  apiVersion: 1 | 2 | 3,
  settings: ExtensionOptions,
  backends: Data,
  runtime: Data,
): ExtensionHandle {
  const base = makePMFacades(realm.PM, realm);
  const namespaces: Data = {};
  for (const key of new Set([...Object.keys(base), ...Object.keys(backends)]))
    namespaces[key] = mergeDescriptors(base[key], backends[key]);
  const unavailable = (family: string, members: string[]) =>
    mergeDescriptors(required(family, members), backends[family]);
  const readStorage = (owner: string): Data => {
    if (!realm.PM.store?.get)
      throw new HostError(
        "Unavailable",
        "PM.store.get capability is not attached",
      );
    const value = realm.PM.store.get("ext." + owner, {});
    return value && typeof value === "object" && !Array.isArray(value)
      ? value
      : {};
  };
  const writeStorage = (owner: string, value: Data): void => {
    if (!realm.PM.store?.set)
      throw new HostError(
        "Unavailable",
        "PM.store.set capability is not attached",
      );
    realm.PM.store.set("ext." + owner, value);
  };
  const controlAPI = {
    anim: namespaces.anim,
    model: namespaces.model,
    transport: namespaces.transport,
    project: mergeDescriptors(namespaces.project, { get: () => realm.PM.proj }),
    util: namespaces.util,
  };
  namespaces.ui = mergeDescriptors(namespaces.ui, {
    controls: mergeDescriptors(
      required("ui.controls", [
        "Segmented",
        "NumField",
        "ColorField",
        "FillField",
        "RampField",
        "PointField",
        "SliderField",
        "Disclosure",
        "FontField",
        "SelectField",
        "TextField",
        "ToggleField",
        "Row",
        "Section",
      ]),
      backends.ui?.controls,
      {
        binding: Object.fromEntries(
          Object.entries(bindings).map(([name, fn]) => [
            name,
            (...args: any[]) => fn(controlAPI, ...args),
          ]),
        ),
      },
    ),
  });
  const deps = {
    ...namespaces,
    pm: realm.PM,
    project: mergeDescriptors(namespaces.project, {
      get: () => realm.PM.proj,
      latest: () => realm.PM.proj,
      revision: () => Number(realm.PM.proj.revision ?? 0),
      apply: realm.edit.apply,
      selection: () => realm.selectionSnapshot(),
      undo: realm.history.undo,
      redo: realm.history.redo,
      snapshot: async (time = realm.PM.time ?? 0, maxWidth = 480) =>
        realm.PM.Export.snapshot(time, maxWidth),
    }),
    history: realm.history,
    edit: realm.edit,
    workspace: mergeDescriptors(namespaces.workspace, realm.workspace),
    render: mergeDescriptors(namespaces.render, {
      rasterAsync: async (...args: any[]) => realm.PM.raster(...args),
      renderFrameToAsync: async (...args: any[]) =>
        realm.PM.renderFrameTo(...args),
      snapshotAsync: async (...args: any[]) =>
        realm.PM.Export.snapshot(...args),
    }),
    assets: unavailable("assets", [
      "pick",
      "import",
      "importUrl",
      "get",
      "readText",
    ]),
    extensions: unavailable("extensions", [
      "list",
      "fork",
      "setEnabled",
      "remove",
      "reload",
      "reveal",
      "requestFix",
      "rebase",
      "setUp",
    ]),
    scene3d: unavailable("scene3d", [
      "isGroup",
      "getView",
      "setView",
      "onViewChange",
      "getNavigationMode",
      "setNavigationMode",
      "onNavigationChange",
      "createNavigation",
      "frame",
      "getMode",
      "setMode",
      "getSpace",
      "setSpace",
      "onGizmoChange",
      "createGizmo",
      "prepareImport",
      "convert",
      "createScene",
      "createObject",
      "createLight",
      "edit",
      "describe",
      "selection",
      "select",
    ]),
    dnd: namespaces.dnd,
    storage: (_id: string) =>
      typeof backends.storageFor === "function"
        ? backends.storageFor(_id)
        : mergeDescriptors(
            {
              get: (key: string) => readStorage(_id)[key],
              set: (key: string, value: unknown) =>
                writeStorage(_id, { ...readStorage(_id), [key]: value }),
              delete: (key: string) => {
                const next = { ...readStorage(_id) };
                delete next[key];
                writeStorage(_id, next);
              },
            },
            backends.storage,
          ),
    panelsBackend: mergeDescriptors(
      required("panels", ["open", "close", "isOpen", "refresh"]),
      { list: () => realm.kernel.panels.ids() },
      backends.panelsBackend,
    ),
    paletteOpen: (query?: string) => {
      if (!backends.palette?.open)
        throw new HostError(
          "Unavailable",
          "palette.open backend is not attached",
        );
      return backends.palette.open(query);
    },
    state: backends.host?.state ?? {
      doc: realm.PM.proj,
      sel: realm.PM.sel,
      transport: backends.transport,
      perf: realm.PM.perf,
    },
    mount: (component: any, target: any, props: Data) => {
      if (backends.host?.mount)
        return backends.host.mount(component, target, props);
      const svelte = runtime.svelte;
      if (!svelte?.mount || !svelte?.flushSync || !svelte?.unmount)
        throw new HostError(
          "Unavailable",
          "Shared Svelte runtime is not attached",
        );
      const instance = svelte.mount(component, { target, props });
      svelte.flushSync();
      let mounted = true;
      return () => {
        if (mounted) {
          mounted = false;
          void svelte.unmount(instance);
        }
      };
    },
    createSubscriber: runtime["svelte/reactivity"]?.createSubscriber,
    haptic: backends.host?.haptic,
    workspaceAppearance: backends.workspaceAppearance,
    themeAppearance: backends.themeAppearance,
    reportRuntimeError: (extensionId: string, error: unknown, label: string) =>
      realm.reportError({ extensionId, label, error }),
  };
  const handle = createExtensionAPI(
    realm.kernel,
    {
      id,
      scope: settings.scope ?? "user",
      manifest: {
        id,
        name: id,
        version: "0.0.0",
        ...backends.manifest,
        ...settings.manifest,
        apiVersion,
      },
    },
    deps,
    settings.vars,
  );
  return {
    api: handle.api,
    setActivating: handle.setActivating,
    dispose: () => handle.disposeAll(),
  };
}
