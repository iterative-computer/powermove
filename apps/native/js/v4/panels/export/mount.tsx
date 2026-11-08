/** Mounts the built-in panel into its window: the same entry a fork of it uses. */
import { mountExportPanel } from "./index";
import type { BuiltinEnv, Mounted } from "../../src/builtin";

export async function mount(env: BuiltinEnv): Promise<Mounted> {
  const { host, transport, rootOptions } = env;
  const p = await mountExportPanel({
    host,
    transport,
    rootOptions,
    anchor: 0,
    subscription: "export",
  });
  // The export panel already has `show()`, which reveals its sheet.
  const mounted: Mounted = p;
  return mounted;
}
