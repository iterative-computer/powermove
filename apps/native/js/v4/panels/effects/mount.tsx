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
      add: (item, target) => [
        {
          command: "effect.add",
          args: { definition: item.id, target: target.id },
        },
      ],
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
