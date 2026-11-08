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
import type { BuiltinEnv, Hot, Mount, Mounted } from "../src/builtin";
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
/** Built-in panel ids (Rust `LIVE_PANELS`) to the module that mounts each. */
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
async function platform(operation: string, input: unknown = null) {
  const id = native("platform.enqueue", { operation, input });
  for (;;) {
    const result = native("platform.poll", id);
    if (result) {
      if (!result.accepted) throw new Error(result.error);
      return result.value;
    }
    await new Promise((resolve) => setTimeout(resolve, 1));
  }
}
/** The realm's host connection and everything a panel mount may use. */
function environment(panel: string): BuiltinEnv {
  const identity = native("identity") as HostIdentity;
  host = new HostClient(identity, async (request) => {
    native("host.enqueue", request);
    for (;;) {
      const ack = native("host.poll", request.requestId);
      if (ack) return ack;
      await new Promise((r) => setTimeout(r, 1));
    }
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
  const env = { ...environment(replaces ?? id), hot: api };
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
  await namespace.activate?.({ host, id, hot: api });
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
    await this.publish();
    return true;
  },
  async write(handle: string, value: Json) {
    await host.command("panel.binding", { handle, value });
    await this.publish();
    return true;
  },
  async show() {
    await mounted?.show?.();
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
