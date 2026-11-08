/** Mounts the built-in panel into its window: the same entry a fork of it uses. */
import { AgentPanel, HostAgentModel } from "./index";
import type { BuiltinEnv, Mounted } from "../../src/builtin";

export async function mount(env: BuiltinEnv): Promise<Mounted> {
  const { host, report, root } = env;
  let agentPublish: ((publication: any) => void) | undefined;
  const model = await HostAgentModel.connect(host, {
    read: "panel.agent.read",
    dispatch: "panel.agent.dispatch",
    subscribe(publish) {
      agentPublish = publish;
      return () => {
        agentPublish = undefined;
      };
    },
  });
  const r = root();
  await r.render(<AgentPanel model={model} onError={report} />);
  const mounted: Mounted = {
    root: r,
    async dispose() {
      model.dispose();
      await r.dispose();
    },
  };
  mounted.settle = () => model.flush();
  const stop = await host.subscribe(
    "agent",
    "project",
    (p) => agentPublish?.(p),
    ["agent"],
  );
  const dispose = mounted.dispose.bind(mounted);
  mounted.dispose = async () => {
    await stop();
    await dispose();
  };
  return mounted;
}
