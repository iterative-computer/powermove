import { createRoot } from "../../src";
import type { BuiltinEnv, Mounted } from "../../src/builtin";
import { ColorModel } from "./model";
import { ColorPanel } from "./panel";

export async function mount(env: BuiltinEnv): Promise<Mounted> {
  const { platform, host, transport, rootOptions, report } = env;
  const model = new ColorModel(host, {
    contextMenu: (items) => platform("contextMenu", items),
    report,
  });
  const root = createRoot(transport, report, rootOptions);
  const command = (name: string) => {
    void platform("command", { name, args: [] }).catch(report);
  };
  try {
    await model.start("color");
    await root.render(<ColorPanel controller={model} command={command} />);
  } catch (error) {
    await root.dispose();
    await model.dispose();
    throw error;
  }
  return {
    root,
    settle: () => model.settled(),
    async dispose() {
      await root.dispose();
      await model.dispose();
    },
  };
}
