import { mountExportPanel } from "./index";
import type { BuiltinEnv, Mounted } from "../../src/builtin";

export async function mount(env: BuiltinEnv): Promise<Mounted> {
  const { host, transport, rootOptions } = env;
  return mountExportPanel({
    host,
    transport,
    rootOptions,
    anchor: 0,
    subscription: "export",
  });
}
