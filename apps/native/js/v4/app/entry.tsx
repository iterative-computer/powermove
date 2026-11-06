/** Production V8 worker entry. Every built-in owns a separate extension isolate. */
import * as preact from "preact";
import * as preactHooks from "preact/hooks";
import { createRoot, type SurfaceRoot } from "../src";
import * as ui from "../src/ui";
import * as hooks from "../src/hooks";
import * as jsxRuntime from "../src/jsx-runtime";
import {
  HostClient,
  type HostIdentity,
  type HostPublication,
} from "../src/host-api";
import { type Json, type NativeEvent, type Transport } from "../src/protocol";
import { mountEffectControls } from "../panels/effect-controls";
import { createEffectsExtension } from "../panels/effects";
import { mountTimeline } from "../panels/timeline";
import { mountInspector, InspectorSession } from "../panels/inspector";
import { mountViewer } from "../panels/viewer";
import { ProjectPanel, ProjectService } from "../panels/project";
import { AgentPanel, HostAgentModel } from "../panels/agent";
import { SettingsPanel, SettingsModel } from "../panels/settings";
import { generalRows } from "../panels/settings/catalogue";
import { mountExportPanel } from "../panels/export";
import type { Registry, Target } from "../panels/effects/model";
declare const runtime: { engine(operation: string, input?: Json): Json };
const native = (operation: string, input: unknown = null): any =>
  runtime.engine(operation, input as Json);
const report = (error: unknown) => native("report", String(error));
interface Mount {
  root: SurfaceRoot;
  dispose(): Promise<void> | void;
}
let mounted: Mount | undefined;
let host: HostClient;
let settleModel: () => Promise<unknown> = async () => {};
let updateModel: (() => void) | undefined;
let show: (() => Promise<unknown>) | undefined;
let registryPublish: ((registry: Registry) => void) | undefined;
let agentPublish: ((publication: any) => void) | undefined;
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
async function mount(panel: string) {
  if (mounted) throw new Error("Panel already mounted");
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
  const command = async <T = Json | undefined,>(
    name: string,
    args: unknown = {},
  ): Promise<T> => (await host.command(name, args as Json)).value as T;
  const fields = (names: string[]) => native("fields.register", names);
  const activatePanel = (id: string) => {
    void platform("activatePanel", id).catch(report);
  };
  const root = () => createRoot(transport, report, rootOptions);
  const settingsPublication = (
    publication: import("../panels/settings").Publication,
  ) => {
    const general = generalRows(publication.multiWindow);
    const leases = fields(
      general.filter((row) => row.profileKey).map((row) => row.id),
    );
    return {
      ...publication,
      rows: [
        ...(publication.rows ?? []),
        ...general
          .filter((row) => row.profileKey)
          .map((row) => ({ ...row, binding: leases[row.id] })),
      ],
    };
  };
  if (panel === "layer-effects") {
    const p = await mountEffectControls({
      transport,
      host,
      rootOptions,
      subscriptionId: "effect-controls",
      rowHeight: native("tokens.row"),
      platform: {
        effectMenu: () => platform("effectMenu"),
        contextMenu: (items) => platform("contextMenu", items),
        activatePanel,
        focus: (id) => {
          void platform("focus", id).catch(report);
        },
        announce: (message) => {
          void platform("announce", message).catch(report);
        },
        report,
      },
    });
    mounted = p;
    settleModel = () => p.controller.settled();
  } else if (panel === "fxbrowser") {
    mounted = await createEffectsExtension({
      transport,
      host,
      rootOptions,
      bindings: fields(["search", "scope"]),
      subscriptionPrefix: "effects",
      viewport: { start: 0, size: 32 },
      onError: report,
      adapter: {
        registry: {
          subscribe(publish) {
            publish(native("effects.registry"));
            registryPublish = publish;
            return () => {
              registryPublish = undefined;
            };
          },
        },
        target: () => native("effects.target") as Target | null,
        add: (item, target) => [
          {
            command: "effect.add",
            args: { definition: item.id, target: target.id },
          },
        ],
        reveal: (_item, target) => ({
          command: "effect.reveal",
          args: { target: target.id },
        }),
      },
    });
  } else if (panel === "timeline") {
    const p = await mountTimeline({
      transport,
      host,
      instance: "timeline",
      window: { first: 0, visible: 32 },
      onError: report,
      retireSurface: async () => {},
      model: {
        inspect: async (window) => native("timeline.frame", window),
        act: async (intent) => {
          await command("panel.timeline", intent);
        },
        dispose: async () => {
          await command("panel.timeline.dispose");
        },
      },
    });
    mounted = p;
    settleModel = () => p.settled();
  } else if (panel === "inspector") {
    let snapshot: any = {
      target: { kind: "no-project" },
      sections: [],
      time: 0,
      playing: false,
    };
    const session = new InspectorSession(
      host,
      {
        update(query, publication) {
          if (query === "evaluated") snapshot = publication;
        },
        inspect: () => snapshot,
        plan: (intent) => ({
          type: "command",
          command: "panel.inspector",
          args: intent as unknown as Json,
        }),
      },
      { request: (request) => platform("inspector", request) },
      {
        schedule(callback) {
          let active = true;
          queueMicrotask(() => {
            if (active) callback();
          });
          return () => {
            active = false;
          };
        },
      },
      report,
    );
    mounted = await mountInspector(session, transport, {
      retire: async () => {},
    });
    settleModel = () => session.settled();
  } else if (panel === "viewer") {
    mounted = await mountViewer({
      host,
      transport,
      rootOptions,
      subscription: "viewer",
      open: () => {
        void platform("command", { name: "open", args: [] }).catch(report);
      },
      onError: report,
    });
  } else if (panel === "assets") {
    const service = new ProjectService(host, {
      command: "panel.project",
      subscription: "project",
      projectStamp: (p: any) => p.stamp ?? null,
    });
    await service.page(0, 96);
    await service.start();
    const r = root();
    await r.render(
      <ProjectPanel
        service={service}
        layout={native("project.layout")}
        native={{
          ...fields(["searchBinding", "renameBinding"]),
          activatePanel,
          menu(entries, choose, context) {
            void platform("project.menu", { entries, context })
              .then((index) => {
                if (index !== null) choose(entries[index]);
              })
              .catch(report);
          },
        }}
      />,
    );
    mounted = {
      root: r,
      async dispose() {
        await service.dispose();
        await r.dispose();
      },
    };
    settleModel = () => service.idle();
  } else if (panel === "agent") {
    const model = await HostAgentModel.connect(host, {
      read: "panel.agent.read",
      dispatch: "panel.agent.dispatch",
      subscribe(publish) {
        agentPublish = publish;
        return () => {
          agentPublish = undefined;
        };
      },
    });
    const r = root();
    await r.render(<AgentPanel model={model} onError={report} />);
    mounted = {
      root: r,
      async dispose() {
        model.dispose();
        await r.dispose();
      },
    };
    settleModel = () => model.flush();
    const stop = await host.subscribe(
      "agent",
      "project",
      (p) => agentPublish?.(p),
      ["agent"],
    );
    const dispose = mounted.dispose.bind(mounted);
    mounted.dispose = async () => {
      await stop();
      await dispose();
    };
  } else if (panel === "settings") {
    const model = new SettingsModel(
      host,
      settingsPublication(
        (await host.query("project", ["settings"]))
          .value as unknown as import("../panels/settings").Publication,
      ),
      {
        present: (section) => {
          void platform("settings.present", section).catch(report);
        },
        closeAndRestoreFocus: () => {
          void platform("settings.close").catch(report);
        },
        title: (section) => {
          void platform("settings.title", section).catch(report);
        },
        scrollTo: (section, options) => {
          void platform("settings.scroll", { section, options }).catch(report);
        },
        focusSearch: () => {
          void platform("focus", "settings.search").catch(report);
        },
        announce: (message) => {
          void platform("announce", message).catch(report);
        },
      },
      {
        perform: async (action, revision) =>
          command<import("../panels/settings").Publication>("panel.settings", {
            action,
            revision,
          }),
      },
      {
        scope: "kernel-profile",
        getSetting: async (key) => ({
          ...host.identity,
          version: 1,
          requestId: 0,
          revision: 0,
          accepted: true,
          ...(await command<{ value?: Json; stateRevision: number }>(
            "profile.get",
            { key },
          )),
        }),
        setSetting: async (key, value) => ({
          ...host.identity,
          version: 1,
          requestId: 0,
          revision: 0,
          accepted: true,
          ...(await command<{ stateRevision: number }>("profile.set", {
            key,
            value,
          })),
        }),
      },
    );
    await model.hydrate();
    const r = root();
    await r.render(
      <SettingsPanel
        model={model}
        bindings={fields(["search", "extensionSearch", "extensionType"])}
      />,
    );
    mounted = {
      root: r,
      async dispose() {
        model.dispose();
        await r.dispose();
      },
    };
    const stop = await host.subscribe(
      "settings",
      "project",
      (p) =>
        model.publish(
          settingsPublication(
            p as unknown as import("../panels/settings").Publication,
          ),
        ),
      ["settings"],
    );
    const dispose = mounted.dispose.bind(mounted);
    mounted.dispose = async () => {
      await stop();
      await dispose();
    };
    show = async () => {
      model.show();
      await r.flush();
    };
  } else if (panel === "export") {
    const p = await mountExportPanel({
      host,
      transport,
      rootOptions,
      anchor: 0,
      subscription: "export",
    });
    mounted = p;
    show = () => p.show();
  } else throw new Error(`Unregistered built-in panel: ${panel}`);
  return true;
}
declare const __linkModules: (graph: string) => [Promise<unknown>, any];
/** A third-party surface extension: `export function view()`, same root as a panel. */
async function mountExtension(id: string, text: string) {
  if (mounted) throw new Error("Panel already mounted");
  const identity = native("identity") as HostIdentity;
  host = new HostClient(identity, async (request) => {
    native("host.enqueue", request);
    for (;;) {
      const ack = native("host.poll", request.requestId);
      if (ack) return ack;
      await new Promise((r) => setTimeout(r, 1));
    }
  });
  const table: Record<string, any> = {
    preact,
    "preact/hooks": preactHooks,
    "powermove/ui": ui,
    "powermove/hooks": hooks,
    "powermove/jsx-runtime": jsxRuntime,
    "powermove/jsx-dev-runtime": jsxRuntime,
  };
  (globalThis as any).__powermove_runtime = table;
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
        if (!(module in table)) throw new Error(`Unsupported import: ${module}`);
        sources[url(target)] = [
          `const m = globalThis.__powermove_runtime[${JSON.stringify(module)}];`,
          ...Object.keys(table[module])
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
  if (typeof namespace.view !== "function")
    throw new Error("A surface extension exports view()");
  const root = createRoot(
    async (commit) => native("surface.commit", commit),
    report,
    { generation: identity.generation },
  );
  await namespace.activate?.({ host, id });
  await root.render(namespace.view());
  await root.flush();
  mounted = { root, dispose: () => root.dispose() };
  return true;
}
(globalThis as any).__pmBuiltin = {
  mount,
  mountExtension,
  async publish() {
    for (const p of native("host.publications") as HostPublication[])
      host.publish(p);
    registryPublish?.(native("effects.registry") as Registry);
    updateModel?.();
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
    await show?.();
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
  await settleModel();
  await mounted?.root.flush();
}
