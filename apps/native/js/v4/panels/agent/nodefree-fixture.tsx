import { createRoot } from "../../src";
import { AgentView } from "./view";
import { fixture } from "./fixtures";
const root = createRoot(async (c) => ({
  accepted: true,
  revision: c.baseRevision + 1,
}));
Object.assign(globalThis, {
  agentProbe: (async () => {
    await root.render(<AgentView publication={fixture()} send={() => {}} />);
    const nodes = root.inspect().nodes;
    const result = {
      panel: nodes.some((n) => n.props.label === "Agent conversation"),
      process: typeof process,
      window: typeof window,
      document: typeof document,
      buffer: typeof Buffer,
    };
    await root.dispose();
    return result;
  })(),
});
