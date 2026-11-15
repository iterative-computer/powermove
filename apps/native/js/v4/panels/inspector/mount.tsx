import { EffectsController } from "../effect-controls/controller";
import { mountInspector, InspectorSession } from "./index";
import type { BuiltinEnv, Mounted } from "../../src/builtin";
import type { Json } from "../../src/protocol";

export async function mount(env: BuiltinEnv): Promise<Mounted> {
  const { platform, host, transport, report } = env;
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
  const controller = new EffectsController(
    host,
    {
      effectMenu: () => platform("effectMenu"),
      contextMenu: (items) => platform("contextMenu", items),
      focus: (id) => {
        void platform("focus", id).catch(report);
      },
      announce: (message) => {
        void platform("announce", message).catch(report);
      },
      command: (name) => {
        void platform("command", { name, args: [] }).catch(report);
      },
      report,
    },
    "inspector:effects",
  );
  await controller.start();
  let mounted: Mounted;
  try {
    mounted = await mountInspector(session, transport, {
      retire: async () => {
        await controller.dispose();
      },
      effects: { controller, rowHeight: env.native("tokens.row") },
    });
  } catch (error) {
    await controller.dispose();
    throw error;
  }
  mounted.settle = async () => {
    await session.settled();
    await controller.settled();
  };
  return mounted;
}
