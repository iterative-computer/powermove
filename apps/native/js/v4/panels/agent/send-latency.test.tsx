import { expect, test } from "bun:test";
import { createRoot, type Commit } from "../../src";
import { AgentView } from "./view";
import { fixture } from "./fixtures";

test("streaming patches keep historical prose nodes and avoid republishing their markup", async () => {
  const commits: Commit[] = [];
  const root = createRoot(async (commit) => {
    commits.push(structuredClone(commit));
    return { accepted: true, revision: commit.baseRevision + 1 };
  });
  const p = fixture();
  p.snapshot.rows.items = Array.from({ length: 20 }, (_, i) => ({
    id: `history:${i}`,
    commandId: "",
    chosen: {},
    open: false,
    value: {
      role: "assistant",
      text: `History ${i}\n\n` + "A **formatted** paragraph.\n\n".repeat(5),
    },
  }));
  p.snapshot.rows.items.push({
    id: "live:text-1",
    commandId: "text-1",
    chosen: {},
    open: false,
    value: { kind: "text", text: "First token" },
  });
  p.snapshot.rows.count = 21;
  p.snapshot.rows.range = [0, 21];
  const send = () => {};
  await root.render(<AgentView publication={p} send={send} />);
  const initial = JSON.stringify(commits).length;
  const nodes = root
    .inspect()
    .nodes.filter((n) => n.text?.startsWith("History"))
    .map((n) => n.id);
  commits.length = 0;
  const next = structuredClone(p);
  next.sequence++;
  next.snapshot.rows.items[20]!.value.text = "First token, then another";
  await root.render(<AgentView publication={next} send={send} />);
  expect(JSON.stringify(commits).length).toBeLessThan(initial / 2);
  expect(
    root
      .inspect()
      .nodes.filter((n) => n.text?.startsWith("History"))
      .map((n) => n.id),
  ).toEqual(nodes);
  expect(
    root.inspect().nodes.some((n) => n.text?.includes("then another")),
  ).toBe(true);
  await root.dispose();
});
