/** Mounts the built-in panel into its window: the same entry a fork of it uses. */
import { mountTimeline } from "./extension";
import type { BuiltinEnv, Mounted } from "../../src/builtin";

interface MenuRow {
  id: string;
  label: string;
  intent: unknown;
}
export async function mount(env: BuiltinEnv): Promise<Mounted> {
  const { native, platform, host, transport, command, report } = env;
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
        if (intent.type === "open") {
          await platform("timeline.effect", { kind: "command", name: "open" });
        } else if (
          intent.type === "graphOptions" ||
          (intent.type === "switch" && intent.name === "parent")
        ) {
          const rows = await command<MenuRow[]>("panel.timeline.menu", intent);
          const choice = await platform("contextMenu", rows);
          const row = rows.find((r) => r.id === choice);
          if (row) await command("panel.timeline", row.intent);
        } else await command("panel.timeline", intent);
      },
      dispose: async () => {
        await command("panel.timeline.dispose");
      },
    },
  });
  const mounted: Mounted = p;
  mounted.settle = () => p.settled();
  return mounted;
}
