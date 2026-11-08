/** Mounts the built-in panel into its window: the same entry a fork of it uses. */
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
  const mounted: Mounted = await mountInspector(session, transport, {
    retire: async () => {},
  });
  mounted.settle = () => session.settled();
  return mounted;
}
