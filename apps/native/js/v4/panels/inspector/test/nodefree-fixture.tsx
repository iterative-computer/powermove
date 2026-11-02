import { mountInspector } from "../index";
import { fixture } from "./fixture";
const f = fixture();
(
  globalThis as unknown as { inspectorResult: Promise<unknown> }
).inspectorResult = (async () => {
  const panel = await mountInspector(
    f.session,
    async (commit) => ({
      accepted: true,
      revision: commit.baseRevision + 1,
    }),
    { retire: async () => {} },
  );
  const nodes = panel.root.inspect().nodes;
  await panel.dispose();
  return {
    title: nodes.find((node) => node.props.id === "inspector")?.props.label,
    value: nodes.find((node) => node.props.id === "value:opacity:0")?.props
      .value,
    controls: nodes.filter((node) => node.props.binding).length,
  };
})();
