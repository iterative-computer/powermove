/** Headless controller + native op transport. Measures JS slices, excluding
 * AppKit layout/GPU. Optional PM_AGENT_VIEW / PM_AGENT_HOST select baseline. */
import { createElement } from "preact";
import { createRoot } from "../../src";
import { HostClient } from "../../src/host-api";
import { fixture } from "./fixtures";
import type { Publication } from "./contract";
const { AgentView } = await import(process.env.PM_AGENT_VIEW || "./view");
const { HostAgentModel } = await import(process.env.PM_AGENT_HOST || "./host");
let publish: (p: Publication) => void = () => {};
const p = fixture();
p.snapshot.rows.items = Array.from({ length: 63 }, (_, i) => ({
  id: `history:${i}`,
  commandId: "",
  chosen: {},
  open: false,
  value: {
    role: i % 2 ? "assistant" : "user",
    text:
      `Message ${i}\n\n` +
      "A **synthetic** reply with `code` and a [link](https://example.invalid).\n\n".repeat(
        6,
      ),
  },
}));
p.snapshot.rows.items.push({
  id: "live:text-1",
  commandId: "text-1",
  chosen: {},
  open: false,
  value: { kind: "text", text: "" },
});
p.snapshot.rows.count = 64;
p.snapshot.rows.range = [0, 64];
const client = new HostClient(
  { realm: "bench", project: p.snapshot.projectId, generation: 1 },
  async (request) => ({
    ...request,
    accepted: true,
    revision: 0,
    value:
      request.operation.type === "command" &&
      request.operation.command === "read"
        ? (p as any)
        : null,
  }),
);
const model = await HostAgentModel.connect(client, {
  read: "read",
  dispatch: "send",
  subscribe(fn: (p: Publication) => void) {
    publish = fn;
    return () => {};
  },
});
let bytes = 0;
const root = createRoot(async (commit) => {
  bytes += JSON.stringify(commit).length;
  return { accepted: true, revision: commit.baseRevision + 1 };
});
const draw = () =>
  root.render(
    createElement(AgentView, { publication: model.inspect(), send: () => {} }),
  );
await draw();
const frames: { ms: number; stack: string; bytes: number }[] = [];
async function measure(stack: string, fn: () => Promise<unknown>) {
  const before = bytes,
    start = performance.now();
  await fn();
  frames.push({ ms: performance.now() - start, stack, bytes: bytes - before });
}
await measure("HostAgentModel.dispatch(submit)", () =>
  model.dispatch({ type: "submit" }),
);
for (let i = 0; i < 24; i++) {
  await measure(
    "publish → inspect → AgentView → Prose/markdown → native op commit",
    async () => {
      p.sequence++;
      p.snapshot.rows.items[63]!.value.text += "token ";
      publish(p);
      await draw();
    },
  );
  await measure("composer draft dispatch during stream", () =>
    model.dispatch({
      type: "draft",
      text: `typing ${i}`,
      selection: [0, 0],
      base: "",
    }),
  );
  await Bun.sleep(16);
}
const worst = frames.reduce((a, b) => (a.ms > b.ms ? a : b));
console.log(
  JSON.stringify(
    {
      frames: frames.length,
      worst,
      streamMaxMs: Math.max(
        ...frames.filter((f) => f.stack.startsWith("publish")).map((f) => f.ms),
      ),
      typingMaxMs: Math.max(
        ...frames
          .filter((f) => f.stack.startsWith("composer"))
          .map((f) => f.ms),
      ),
      totalCommitBytes: bytes,
    },
    null,
    2,
  ),
);
await root.dispose();
model.dispose();
client.retire();
