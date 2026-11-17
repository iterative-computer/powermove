import { createEffectsExtension } from "./index";
import type { Registry, Target } from "./model";
import type { BuiltinEnv, Mounted } from "../../src/builtin";

export async function mount(env: BuiltinEnv): Promise<Mounted> {
  const { native, host, transport, rootOptions, fields, report } = env;
  let registryPublish: ((registry: Registry) => void) | undefined;
  const mounted: Mounted = await createEffectsExtension({
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
      // A transition goes on the selection's In, as v1's Add did.
      add: (item, target) => {
        const args: Record<string, string> = {
          definition: item.id,
          target: target.id,
        };
        if (item.kind !== "transition")
          return [{ command: "effect.add", args }];
        args.edge = "in";
        return [{ command: "transition.set", args }];
      },
      reveal: (_item, target) => ({
        command: "effect.reveal",
        args: { target: target.id },
      }),
    },
  });
  mounted.publish = () =>
    registryPublish?.(native("effects.registry") as Registry);
  return mounted;
}
