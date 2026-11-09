import { mountViewer } from "./index";
import type { BuiltinEnv, Mounted } from "../../src/builtin";

export async function mount(env: BuiltinEnv): Promise<Mounted> {
  const { platform, host, transport, rootOptions, report } = env;
  return mountViewer({
    host,
    transport,
    rootOptions,
    subscription: "viewer",
    open: () => {
      void platform("command", { name: "open", args: [] }).catch(report);
    },
    onError: report,
  });
}
