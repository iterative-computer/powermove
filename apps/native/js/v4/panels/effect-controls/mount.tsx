/** Mounts the built-in panel into its window: the same entry a fork of it uses. */
import { mountEffectControls } from "./index";
import type { BuiltinEnv, Mounted } from "../../src/builtin";

export async function mount(env: BuiltinEnv): Promise<Mounted> {
  const {
    native,
    platform,
    host,
    transport,
    rootOptions,
    report,
  } = env;
  const p = await mountEffectControls({
    transport,
    host,
    rootOptions,
    subscriptionId: "effect-controls",
    rowHeight: native("tokens.row"),
    platform: {
      effectMenu: () => platform("effectMenu"),
      contextMenu: (items) => platform("contextMenu", items),
      focus: (id) => {
        void platform("focus", id).catch(report);
      },
      announce: (message) => {
        void platform("announce", message).catch(report);
      },
      report,
    },
  });
  const mounted: Mounted = p;
  mounted.settle = () => p.controller.settled();
  return mounted;
}
