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
    // The app's own commands (transport, view toggles): one command and one
    // state with the menus, the keymap and the timeline's header.
    command: (name: string) => {
      void platform("command", { name, args: [] }).catch(report);
    },
    onError: report,
  });
}
