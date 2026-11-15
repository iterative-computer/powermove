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
    {
      request: async (request) => {
        const choice = await platform("inspector", request);
        if (choice === "save-preset") {
          const row =
            request.type === "row-menu" ? session.row(request.row) : undefined;
          const control = row?.control;
          const paths =
            control && "binding" in control ? control.binding.channels : [];
          await platform("command", {
            name: "saveAnimationPreset",
            args: [{ properties: paths }],
          });
          return null;
        }
        return choice;
      },
    },
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
  const mounted: Mounted = await mountInspector(session, transport, {
    retire: async () => {},
  });
  mounted.settle = () => session.settled();
  return mounted;
}
